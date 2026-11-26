//! Two-file comparison (Tools > Compare Files).
//!
//! Text: a line diff (patience anchors on lines unique to both sides, Myers between them) laid
//! out as side-by-side rows, with moved-line detection: a changed line whose text appears in a
//! *different* change on the other side is marked as moved there.
//!
//! Binary: a positional byte comparison plus moved-block detection: runs of at least
//! `MIN_MOVED_BLOCK` bytes found at a different offset in the other file.
//!
//! The moved-line rule and the binary relocation search are simplified from OxideMerge
//! (MIT, same author). Nothing here touches the UI.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

/// How a side-by-side row differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Same,
    /// Both sides have a line, and they differ.
    Changed,
    /// Only the left side has a line.
    Deleted,
    /// Only the right side has a line.
    Inserted,
}

/// One row of the side-by-side text comparison. Line numbers are 0-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub kind: RowKind,
    /// The left line was moved: its text is in another change on the right, at this line.
    pub left_moved: Option<usize>,
    /// The right line was moved: its text is in another change on the left, at this line.
    pub right_moved: Option<usize>,
    /// Part of a change of blank lines only, which `TextOptions::ignore_blank_lines` does not
    /// count: shown, but not a difference.
    pub ignored: bool,
}

impl Row {
    fn new(left: Option<usize>, right: Option<usize>, kind: RowKind) -> Self {
        Self { left, right, kind, left_moved: None, right_moved: None, ignored: false }
    }
}

/// What a text comparison does not count as a difference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextOptions {
    /// Changes made only of blank (or whitespace-only) lines.
    pub ignore_blank_lines: bool,
    /// Upper and lower case.
    pub ignore_case: bool,
    /// Line terminator styles (CRLF, LF, CR).
    pub ignore_eol: bool,
}

impl Default for TextOptions {
    fn default() -> Self {
        Self { ignore_blank_lines: false, ignore_case: false, ignore_eol: true }
    }
}

/// A line's terminator, for comparing terminator styles: what follows each line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Eol {
    None,
    Lf,
    CrLf,
    Cr,
}

/// The terminator after each line of `text`, one per line as `EditorState::set_text` splits it
/// (the last line has none).
pub fn line_endings(text: &str) -> Vec<Eol> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                out.push(Eol::CrLf);
                i += 1;
            }
            b'\r' => out.push(Eol::Cr),
            b'\n' => out.push(Eol::Lf),
            _ => {}
        }
        i += 1;
    }
    out.push(Eol::None);
    out
}

/// Counts shown in the compare window's summary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStats {
    pub changed: usize,
    pub deleted: usize,
    pub inserted: usize,
    /// Lines on either side marked as moved.
    pub moved: usize,
}

#[derive(Debug, Default)]
pub struct TextDiff {
    pub rows: Vec<Row>,
    /// Row ranges of each difference block, in order.
    pub hunks: Vec<Range<usize>>,
    /// Row of each left / right line.
    left_rows: Vec<usize>,
    right_rows: Vec<usize>,
}

impl TextDiff {
    pub fn is_identical(&self) -> bool {
        self.hunks.is_empty()
    }

    pub fn stats(&self) -> TextStats {
        let mut s = TextStats::default();
        for r in self.rows.iter().filter(|r| !r.ignored) {
            match r.kind {
                RowKind::Same => {}
                RowKind::Changed => s.changed += 1,
                RowKind::Deleted => s.deleted += 1,
                RowKind::Inserted => s.inserted += 1,
            }
            s.moved += usize::from(r.left_moved.is_some()) + usize::from(r.right_moved.is_some());
        }
        s
    }

    /// For a side shown without filler lines: `bounds[r]` is how many of its lines come before
    /// row `r` (`rows.len() + 1` entries). Rows `s..e` span lines `bounds[s]..bounds[e]` there.
    pub fn line_bounds(&self, right: bool) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.rows.len() + 1);
        let mut n = 0;
        out.push(0);
        for r in &self.rows {
            n += usize::from(if right { r.right.is_some() } else { r.left.is_some() });
            out.push(n);
        }
        out
    }

    /// The row showing a left (`right == false`) or right line.
    pub fn row_of(&self, right: bool, line: usize) -> Option<usize> {
        if right { self.right_rows.get(line).copied() } else { self.left_rows.get(line).copied() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Equal(usize, usize),
    Delete(usize),
    Insert(usize),
}

/// Beyond this many edits Myers gives up on a range between anchors and reports it replaced
/// as a whole, bounding time and the trace's memory (about `MAX_EDITS²` integers).
const MAX_EDITS: usize = 1500;

/// Patience recursion depth limit; deeper ranges go straight to Myers.
const MAX_DEPTH: usize = 64;

/// Compares two documents line by line. Lines are compared exactly.
#[cfg(test)]
pub fn diff_lines(left: &[String], right: &[String]) -> TextDiff {
    diff_lines_with(left, right, [None, None], TextOptions::default())
}

/// Compares two documents line by line under `opts`. `eols` are each side's line terminators
/// ([`line_endings`]), compared unless `opts.ignore_eol`; a side without them counts as
/// having the same terminators as the other.
pub fn diff_lines_with(left: &[String], right: &[String], eols: [Option<&[Eol]>; 2], opts: TextOptions) -> TextDiff {
    // Intern lines (as compared: case folded), so the diff compares integers. Lines are
    // matched on their text alone; terminators are compared afterwards, so a file whose every
    // line ends differently still lines up line for line (as in OxideMerge).
    let mut ids: HashMap<Cow<'_, str>, u32> = HashMap::new();
    let (mut a, mut b) = (Vec::with_capacity(left.len()), Vec::with_capacity(right.len()));
    for (lines, out) in [(left, &mut a), (right, &mut b)] {
        for l in lines {
            let text = if opts.ignore_case { Cow::Owned(l.to_lowercase()) } else { Cow::Borrowed(l.as_str()) };
            let next = ids.len() as u32;
            out.push(*ids.entry(text).or_insert(next));
        }
    }
    let mut ops = Vec::with_capacity(a.len().max(b.len()));
    diff_range(&a, &b, 0, 0, &mut ops, 0, MAX_EDITS);

    let mut out = TextDiff { left_rows: vec![0; a.len()], right_rows: vec![0; b.len()], ..Default::default() };
    let (mut dels, mut ins) = (Vec::new(), Vec::new());
    for op in ops.into_iter().chain(std::iter::once(Op::Equal(usize::MAX, usize::MAX))) {
        match op {
            Op::Delete(i) => dels.push(i),
            Op::Insert(j) => ins.push(j),
            Op::Equal(i, j) => {
                if !dels.is_empty() || !ins.is_empty() {
                    let start = out.rows.len();
                    for k in 0..dels.len().max(ins.len()) {
                        let (l, r) = (dels.get(k).copied(), ins.get(k).copied());
                        let kind = match (l, r) {
                            (Some(_), Some(_)) => RowKind::Changed,
                            (Some(_), None) => RowKind::Deleted,
                            _ => RowKind::Inserted,
                        };
                        out.rows.push(Row::new(l, r, kind));
                    }
                    out.hunks.push(start..out.rows.len());
                    dels.clear();
                    ins.clear();
                }
                if i != usize::MAX {
                    out.rows.push(Row::new(Some(i), Some(j), RowKind::Same));
                }
            }
        }
    }
    for (ri, r) in out.rows.iter().enumerate() {
        if let Some(l) = r.left {
            out.left_rows[l] = ri;
        }
        if let Some(rr) = r.right {
            out.right_rows[rr] = ri;
        }
    }
    if let [Some(le), Some(re)] = eols
        && !opts.ignore_eol
    {
        mark_eol_changes(&mut out, le, re);
    }
    detect_moved(&mut out, left, right, &a, &b);
    if opts.ignore_blank_lines {
        ignore_blank_hunks(&mut out, left, right);
    }
    out
}

/// Marks matched lines whose terminators differ as changed, and gathers the changed rows into
/// hunks again (a terminator change joins the hunks next to it).
fn mark_eol_changes(diff: &mut TextDiff, le: &[Eol], re: &[Eol]) {
    let mut any = false;
    for r in &mut diff.rows {
        if let (RowKind::Same, Some(l), Some(rr)) = (r.kind, r.left, r.right)
            && le.get(l).copied().unwrap_or(Eol::None) != re.get(rr).copied().unwrap_or(Eol::None)
        {
            r.kind = RowKind::Changed;
            any = true;
        }
    }
    if any {
        diff.hunks.clear();
        let mut start = None;
        for (i, r) in diff.rows.iter().enumerate().chain(std::iter::once((diff.rows.len(), &Row::new(None, None, RowKind::Same)))) {
            match (r.kind == RowKind::Same, start) {
                (false, None) => start = Some(i),
                (true, Some(s)) => {
                    diff.hunks.push(s..i);
                    start = None;
                }
                _ => {}
            }
        }
    }
}

/// Drops the changes made only of blank lines from the hunks, marking their rows ignored.
fn ignore_blank_hunks(diff: &mut TextDiff, left: &[String], right: &[String]) {
    let blank = |r: &Row| r.left.is_none_or(|l| left[l].trim().is_empty()) && r.right.is_none_or(|l| right[l].trim().is_empty());
    let rows = &mut diff.rows;
    diff.hunks.retain(|h| {
        if rows[h.clone()].iter().all(blank) {
            rows[h.clone()].iter_mut().for_each(|r| r.ignored = true);
            false
        } else {
            true
        }
    });
}

/// Longer lines are highlighted as a whole rather than word by word.
const MAX_INLINE_BYTES: usize = 4000;
/// Word diffs run for every changed line scrolled into view, so they get a much smaller edit
/// budget than line diffs (about `(N + M) · 200` steps); past it the differing middle of the
/// line is highlighted as a whole.
const MAX_INLINE_EDITS: usize = 200;

/// Word-level differences inside a changed line pair: the byte ranges of `a` and of `b` that
/// are not shared. Words are runs of letters, digits and `_` (each CJK character on its own),
/// runs of spaces, or single other characters, as in OxideMerge's word mode.
pub fn inline_diff(a: &str, b: &str) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let whole = |s: &str| if s.is_empty() { Vec::new() } else { std::iter::once(0..s.len()).collect() };
    if a.len() > MAX_INLINE_BYTES || b.len() > MAX_INLINE_BYTES {
        return (whole(a), whole(b));
    }
    let (ta, tb) = (words(a), words(b));
    let mut ids: HashMap<&str, u32> = HashMap::new();
    let (mut ia, mut ib) = (Vec::with_capacity(ta.len()), Vec::with_capacity(tb.len()));
    for (s, tokens, out) in [(a, &ta, &mut ia), (b, &tb, &mut ib)] {
        for r in tokens {
            let next = ids.len() as u32;
            out.push(*ids.entry(&s[r.clone()]).or_insert(next));
        }
    }
    let mut ops = Vec::new();
    diff_range(&ia, &ib, 0, 0, &mut ops, 0, MAX_INLINE_EDITS);
    let (mut ra, mut rb) = (Vec::new(), Vec::new());
    for op in ops {
        match op {
            Op::Delete(i) => push_merged(&mut ra, ta[i].clone()),
            Op::Insert(j) => push_merged(&mut rb, tb[j].clone()),
            Op::Equal(..) => {}
        }
    }
    (ra, rb)
}

fn push_merged(v: &mut Vec<Range<usize>>, r: Range<usize>) {
    match v.last_mut() {
        Some(last) if last.end == r.start => last.end = r.end,
        _ => v.push(r),
    }
}

/// Splits a line into word tokens (byte ranges).
fn words(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    while let Some((start, c)) = it.next() {
        let mut end = start + c.len_utf8();
        let same: Option<fn(char) -> bool> = if is_word(c) {
            Some(is_word)
        } else if c.is_whitespace() {
            Some(char::is_whitespace)
        } else {
            None
        };
        if let Some(same) = same {
            while let Some(&(i, n)) = it.peek().filter(|(_, n)| same(*n)) {
                end = i + n.len_utf8();
                it.next();
            }
        }
        out.push(start..end);
    }
    out
}

/// A word character. CJK and other wide scripts put no spaces between words, so each of their
/// characters is a word of its own.
fn is_word(c: char) -> bool {
    (c.is_alphanumeric() && (c as u32) < 0x2E80) || c == '_'
}

/// Marks changed lines whose text appears in a different hunk on the other side. Blank lines
/// are never moved: they match anywhere and would only add noise.
fn detect_moved(diff: &mut TextDiff, left: &[String], right: &[String], a: &[u32], b: &[u32]) {
    // For each side, the first changed line with each id in each hunk: id -> [(hunk, line)].
    let mut places: [HashMap<u32, Vec<(usize, usize)>>; 2] = Default::default();
    for (h, range) in diff.hunks.iter().enumerate() {
        for r in &diff.rows[range.clone()] {
            for (side, line, ids, text) in [(0, r.left, a, left), (1, r.right, b, right)] {
                if let Some(line) = line.filter(|&l| !text[l].trim().is_empty()) {
                    let list = places[side].entry(ids[line]).or_default();
                    if list.last().is_none_or(|&(lh, _)| lh != h) {
                        list.push((h, line));
                    }
                }
            }
        }
    }
    for (h, range) in diff.hunks.iter().enumerate() {
        for r in &mut diff.rows[range.clone()] {
            let find = |other: &HashMap<u32, Vec<(usize, usize)>>, id: u32| -> Option<usize> {
                let list = other.get(&id)?;
                // Not moved when the other side also has it in this same hunk (a reordering
                // inside one change reads better as a plain change).
                if list.iter().any(|&(oh, _)| oh == h) {
                    return None;
                }
                list.first().map(|&(_, line)| line)
            };
            if let Some(l) = r.left.filter(|&l| !left[l].trim().is_empty()) {
                r.left_moved = find(&places[1], a[l]);
            }
            if let Some(rr) = r.right.filter(|&rr| !right[rr].trim().is_empty()) {
                r.right_moved = find(&places[0], b[rr]);
            }
        }
    }
}

/// Appends the edit script turning `a` into `b`; `ao`/`bo` are their offsets in the whole files.
fn diff_range(a: &[u32], b: &[u32], ao: usize, bo: usize, out: &mut Vec<Op>, depth: usize, max_edits: usize) {
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    out.extend((0..prefix).map(|k| Op::Equal(ao + k, bo + k)));
    let (a, b, ao, bo) = (&a[prefix..], &b[prefix..], ao + prefix, bo + prefix);
    let suffix = a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[..a.len() - suffix], &b[..b.len() - suffix]);

    if a.is_empty() || b.is_empty() {
        out.extend((0..a.len()).map(|i| Op::Delete(ao + i)));
        out.extend((0..b.len()).map(|j| Op::Insert(bo + j)));
    } else {
        let anchors = if depth < MAX_DEPTH { unique_anchors(a, b) } else { Vec::new() };
        if anchors.is_empty() {
            match myers(a, b, max_edits) {
                Some(ops) => out.extend(ops.into_iter().map(|op| match op {
                    Op::Equal(i, j) => Op::Equal(ao + i, bo + j),
                    Op::Delete(i) => Op::Delete(ao + i),
                    Op::Insert(j) => Op::Insert(bo + j),
                })),
                None => {
                    out.extend((0..a.len()).map(|i| Op::Delete(ao + i)));
                    out.extend((0..b.len()).map(|j| Op::Insert(bo + j)));
                }
            }
        } else {
            let (mut pa, mut pb) = (0, 0);
            for (i, j) in anchors {
                diff_range(&a[pa..i], &b[pb..j], ao + pa, bo + pb, out, depth + 1, max_edits);
                out.push(Op::Equal(ao + i, bo + j));
                (pa, pb) = (i + 1, j + 1);
            }
            diff_range(&a[pa..], &b[pb..], ao + pa, bo + pb, out, depth + 1, max_edits);
        }
    }
    let (ae, be) = (ao + a.len(), bo + b.len());
    out.extend((0..suffix).map(|k| Op::Equal(ae + k, be + k)));
}

/// Patience anchors: lines occurring exactly once in each side, kept in a longest increasing
/// subsequence so they pair up in order.
fn unique_anchors(a: &[u32], b: &[u32]) -> Vec<(usize, usize)> {
    // id -> (count in a, count in b, position in a, position in b)
    let mut seen: HashMap<u32, (u32, u32, usize, usize)> = HashMap::new();
    for (i, &id) in a.iter().enumerate() {
        let e = seen.entry(id).or_insert((0, 0, i, 0));
        e.0 += 1;
    }
    for (j, &id) in b.iter().enumerate() {
        if let Some(e) = seen.get_mut(&id) {
            e.1 += 1;
            e.3 = j;
        }
    }
    let mut pairs: Vec<(usize, usize)> = seen.values().filter(|e| e.0 == 1 && e.1 == 1).map(|e| (e.2, e.3)).collect();
    pairs.sort_unstable();
    // Longest increasing subsequence on the b positions (patience sorting).
    let mut tails: Vec<usize> = Vec::new(); // index into pairs of the smallest tail per length
    let mut prev: Vec<Option<usize>> = vec![None; pairs.len()];
    for (k, &(_, bj)) in pairs.iter().enumerate() {
        let pos = tails.partition_point(|&t| pairs[t].1 < bj);
        prev[k] = pos.checked_sub(1).map(|p| tails[p]);
        if pos == tails.len() {
            tails.push(k);
        } else {
            tails[pos] = k;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied();
    while let Some(k) = at {
        out.push(pairs[k]);
        at = prev[k];
    }
    out.reverse();
    out
}

/// Myers' O(ND) diff with a trace for backtracking. `None` when more than `max_edits` edits
/// are needed.
fn myers(a: &[u32], b: &[u32], max_edits: usize) -> Option<Vec<Op>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (a.len() + b.len()).min(max_edits) as isize;
    let off = max + 1;
    let mut v = vec![0isize; (2 * off + 1) as usize];
    // trace[d] holds v[-d-1 ..= d+1] as it was before step d.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut end_d = None;
    'outer: for d in 0..=max {
        trace.push(v[(off - d - 1) as usize..=(off + d + 1) as usize].to_vec());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[(off + k - 1) as usize] < v[(off + k + 1) as usize]) {
                v[(off + k + 1) as usize]
            } else {
                v[(off + k - 1) as usize] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[(off + k) as usize] = x;
            if x >= n && y >= m {
                end_d = Some(d);
                break 'outer;
            }
            k += 2;
        }
    }
    let end_d = end_d?;
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=end_d).rev() {
        let t = &trace[d as usize];
        let get = |k: isize| t[(k + d + 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && get(k - 1) < get(k + 1)) { k + 1 } else { k - 1 };
        let prev_x = get(prev_k);
        let prev_y = prev_x - prev_k;
        let (sx, sy) = if prev_k == k + 1 { (prev_x, prev_x - prev_k + 1) } else { (prev_x + 1, prev_y) };
        while x > sx && y > sy {
            x -= 1;
            y -= 1;
            ops.push(Op::Equal(x as usize, y as usize));
        }
        if prev_k == k + 1 {
            ops.push(Op::Insert(prev_y as usize));
        } else {
            ops.push(Op::Delete(prev_x as usize));
        }
        (x, y) = (prev_x, prev_y);
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        ops.push(Op::Equal(x as usize, y as usize));
    }
    ops.reverse();
    Some(ops)
}

// ── Binary ──

/// Shorter byte matches are usually coincidence rather than moved data.
pub const MIN_MOVED_BLOCK: usize = 16;

/// At most this many moved blocks are reported.
const MAX_MOVED_BLOCKS: usize = 1024;

/// Bytes `source` of the left file found at `target` in the right file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedBlock {
    pub source: Range<usize>,
    pub target: Range<usize>,
}

/// One stretch of the aligned hex layout: display cells `at..at + len`, where each side shows
/// `count[s]` of its bytes from offset `start[s]`, then blank cells. A `matched` stretch is a
/// moved block lined up on both sides (the same bytes at different offsets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub at: usize,
    pub len: usize,
    pub start: [usize; 2],
    pub count: [usize; 2],
    pub matched: bool,
}

#[derive(Debug, Default)]
pub struct BinaryDiff {
    /// Display cells that differ: a byte on one side only, or different bytes.
    pub differing: usize,
    /// Moved blocks, sorted by source offset.
    pub moved: Vec<MovedBlock>,
    /// The aligned layout both hex panes are drawn from, in display order.
    pub stretches: Vec<Stretch>,
}

impl BinaryDiff {
    pub fn is_identical(&self) -> bool {
        self.differing == 0
    }

    /// Number of display cells.
    pub fn cells(&self) -> usize {
        self.stretches.last().map_or(0, |s| s.at + s.len)
    }

    /// The stretch holding display cell `d`.
    pub fn stretch_at(&self, d: usize) -> Option<&Stretch> {
        let i = self.stretches.partition_point(|s| s.at + s.len <= d);
        self.stretches.get(i).filter(|s| s.at <= d)
    }

    /// The byte offset each side shows at display cell `d` (`None`: a blank cell).
    pub fn cell(&self, d: usize) -> [Option<usize>; 2] {
        let Some(s) = self.stretch_at(d) else { return [None, None] };
        let k = d - s.at;
        [0, 1].map(|i| (k < s.count[i]).then(|| s.start[i] + k))
    }

    /// Whether display cell `d` is a difference.
    pub fn cell_differs(&self, a: &[u8], b: &[u8], d: usize) -> bool {
        match (self.stretch_at(d), self.cell(d)) {
            (Some(s), _) if s.matched => false,
            (_, [Some(x), Some(y)]) => a[x] != b[y],
            (_, [None, None]) => false,
            _ => true,
        }
    }

    /// The next run of differing display cells at or after `from`.
    pub fn next_run(&self, a: &[u8], b: &[u8], from: usize) -> Option<Range<usize>> {
        let first = self.stretches.partition_point(|s| s.at + s.len <= from);
        for s in self.stretches.iter().skip(first).filter(|s| !s.matched) {
            let lo = from.max(s.at) - s.at;
            // Both sides have bytes up to `both`; past it every cell differs.
            let both = s.count[0].min(s.count[1]);
            let (x, y) = (&a[s.start[0]..s.start[0] + both], &b[s.start[1]..s.start[1] + both]);
            let start = if lo < both {
                lo + common_prefix(&x[lo..], &y[lo..])
            } else {
                lo
            };
            if start >= s.len {
                continue;
            }
            let mut end = start + 1;
            while end < s.len && self.cell_differs(a, b, s.at + end) {
                end += 1;
            }
            // A run can go on into the next stretch.
            let mut end = s.at + end;
            while end < self.cells() && self.cell_differs(a, b, end) {
                end += 1;
            }
            return Some(s.at + start..end);
        }
        None
    }

    /// The last run of differing display cells ending at or before `before`.
    pub fn prev_run(&self, a: &[u8], b: &[u8], before: usize) -> Option<Range<usize>> {
        let mut at = before.min(self.cells());
        // Skip matched stretches and equal bytes; stretches are short scans at worst here.
        while at > 0 {
            let s = *self.stretch_at(at - 1)?;
            if s.matched {
                at = s.at;
                continue;
            }
            let both = s.count[0].min(s.count[1]);
            let k = at - s.at;
            if k <= both {
                let (x, y) = (&a[s.start[0]..s.start[0] + k], &b[s.start[1]..s.start[1] + k]);
                let same = common_suffix(x, y);
                if same == k {
                    at = s.at;
                    continue;
                }
                at -= same;
            }
            break;
        }
        if at == 0 {
            return None;
        }
        let end = at;
        while at > 0 && self.cell_differs(a, b, at - 1) {
            at -= 1;
        }
        Some(at..end)
    }
}

pub fn diff_bytes(a: &[u8], b: &[u8]) -> BinaryDiff {
    let moved = if a == b { Vec::new() } else { moved_blocks(a, b) };
    let stretches = layout(&aligned_chain(&moved), [a.len(), b.len()]);
    let mut differing = 0;
    for s in stretches.iter().filter(|s| !s.matched) {
        let both = s.count[0].min(s.count[1]);
        let (x, y) = (&a[s.start[0]..s.start[0] + both], &b[s.start[1]..s.start[1] + both]);
        differing += s.len - both;
        for (cx, cy) in x.chunks(4096).zip(y.chunks(4096)) {
            if cx != cy {
                differing += cx.iter().zip(cy).filter(|(p, q)| p != q).count();
            }
        }
    }
    BinaryDiff { differing, moved, stretches }
}

/// The moved blocks that keep their order in both files, longest in total: those that can be
/// lined up side by side (after OxideMerge's aligned chain).
fn aligned_chain(moved: &[MovedBlock]) -> Vec<&MovedBlock> {
    let mut blocks: Vec<&MovedBlock> = moved.iter().collect();
    blocks.sort_unstable_by_key(|m| (m.source.start, m.target.start));
    // best[i]: the longest total of a chain ending with block i, and the block before it.
    let mut best: Vec<(usize, Option<usize>)> = Vec::with_capacity(blocks.len());
    for (i, m) in blocks.iter().enumerate() {
        let before = (0..i).filter(|&j| blocks[j].source.end <= m.source.start && blocks[j].target.end <= m.target.start).max_by_key(|&j| best[j].0);
        best.push((before.map_or(0, |j| best[j].0) + m.source.len(), before));
    }
    let mut at = (0..blocks.len()).max_by_key(|&i| best[i].0);
    let mut chain = Vec::new();
    while let Some(i) = at {
        chain.push(blocks[i]);
        at = best[i].1;
    }
    chain.reverse();
    chain
}

/// The aligned layout: the chained blocks side by side, and between them each side's bytes
/// from the same start, the shorter side padded with blank cells.
fn layout(chain: &[&MovedBlock], lens: [usize; 2]) -> Vec<Stretch> {
    let mut out = Vec::new();
    let mut at = 0;
    let mut next = [0, 0];
    let mut push = |out: &mut Vec<Stretch>, start: [usize; 2], count: [usize; 2], matched: bool| {
        let len = count[0].max(count[1]);
        if len > 0 {
            out.push(Stretch { at, len, start, count, matched });
            at += len;
        }
    };
    for m in chain {
        push(&mut out, next, [m.source.start - next[0], m.target.start - next[1]], false);
        push(&mut out, [m.source.start, m.target.start], [m.source.len(), m.target.len()], true);
        next = [m.source.end, m.target.end];
    }
    push(&mut out, next, [lens[0] - next[0], lens[1] - next[1]], false);
    out
}

/// A cheap fingerprint of a 16-byte window. Exact bytes are verified before extending a
/// candidate, so collisions can only suppress a match.
fn fingerprint(window: &[u8]) -> u64 {
    let first = u64::from_le_bytes(window[..8].try_into().unwrap());
    let second = u64::from_le_bytes(window[8..16].try_into().unwrap());
    (first.wrapping_mul(0x9e37_79b1_85eb_ca87) ^ second.rotate_left(31)).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count()
}

/// Exact relocations: windows unique in both files at different offsets, extended both ways.
/// Seeds from the left file are sampled to bound memory, so short moves in very large files
/// may be missed.
fn relocated_blocks(left: &[u8], right: &[u8]) -> Vec<MovedBlock> {
    #[derive(Clone, Copy)]
    struct Seed {
        left: usize,
        unique: bool,
        right: Option<usize>,
        repeated: bool,
    }
    if left.len().min(right.len()) < MIN_MOVED_BLOCK {
        return Vec::new();
    }
    const MAX_SEEDS: usize = 65_536;
    let windows = left.len() - MIN_MOVED_BLOCK + 1;
    let stride = windows.div_ceil(MAX_SEEDS);
    let mut seeds: HashMap<u64, Seed> = HashMap::with_capacity(windows.div_ceil(stride));
    for (i, w) in left.windows(MIN_MOVED_BLOCK).enumerate().step_by(stride) {
        // A window still at its own offset is not a relocation.
        if right.get(i..i + MIN_MOVED_BLOCK) == Some(w) {
            continue;
        }
        let s = seeds.entry(fingerprint(w)).or_insert(Seed { left: i, unique: true, right: None, repeated: false });
        s.unique &= s.left == i;
    }
    seeds.retain(|_, s| s.unique);
    if seeds.is_empty() {
        return Vec::new();
    }
    // A bit filter skips most hash-table probes for unrelated windows.
    let bits = (seeds.len() * 16).next_power_of_two();
    let mut filter = vec![0u64; bits.div_ceil(64)];
    let slot = |h: u64| ((h >> 32) ^ h) as usize & (bits - 1);
    for &h in seeds.keys() {
        filter[slot(h) / 64] |= 1 << (slot(h) % 64);
    }
    let probe = |filter: &[u64], h: u64| filter[slot(h) / 64] & (1 << (slot(h) % 64)) != 0;
    for (j, w) in right.windows(MIN_MOVED_BLOCK).enumerate() {
        let h = fingerprint(w);
        if probe(&filter, h)
            && let Some(s) = seeds.get_mut(&h)
            && s.right.replace(j).is_some()
        {
            s.repeated = true;
        }
    }
    seeds.retain(|_, s| !s.repeated && s.right.is_some_and(|r| r != s.left));
    if stride > 1 && !seeds.is_empty() {
        // Sampling skipped windows: make sure each seed is unique in all of the left file.
        for (i, w) in left.windows(MIN_MOVED_BLOCK).enumerate() {
            let h = fingerprint(w);
            if probe(&filter, h)
                && let Some(s) = seeds.get_mut(&h)
            {
                s.unique &= s.left == i;
            }
        }
    }
    let mut anchors: Vec<(usize, usize)> = seeds.values().filter(|s| s.unique).filter_map(|s| Some((s.left, s.right?))).collect();
    anchors.sort_unstable();

    // The free interval around `at` in one file, or None if a block already covers it.
    fn bounds(used: &BTreeMap<usize, usize>, at: usize, len: usize) -> Option<(usize, usize)> {
        let lo = used.range(..=at).next_back().map_or(0, |(_, &end)| end);
        let hi = used.range(at..).next().map_or(len, |(&start, _)| start);
        (lo <= at && at + MIN_MOVED_BLOCK <= hi).then_some((lo, hi))
    }
    let (mut used_l, mut used_r) = (BTreeMap::new(), BTreeMap::new());
    let mut out = Vec::new();
    for (a, b) in anchors {
        let Some((lo_a, hi_a)) = bounds(&used_l, a, left.len()) else { continue };
        let Some((lo_b, hi_b)) = bounds(&used_r, b, right.len()) else { continue };
        if left[a..a + MIN_MOVED_BLOCK] != right[b..b + MIN_MOVED_BLOCK] {
            continue;
        }
        let back = common_suffix(&left[lo_a..a], &right[lo_b..b]);
        let fwd = common_prefix(&left[a + MIN_MOVED_BLOCK..hi_a], &right[b + MIN_MOVED_BLOCK..hi_b]);
        let (mut s, mut t, mut len) = (a - back, b - back, MIN_MOVED_BLOCK + back + fwd);
        // Remember even runs rejected below, so anchors inside one long run are skipped.
        used_l.insert(s, s + len);
        used_r.insert(t, t + len);
        // Trim ends that are also unchanged at their own offsets in both directions.
        while len > 0 && right.get(s) == Some(&left[s]) && left.get(t) == Some(&right[t]) {
            (s, t, len) = (s + 1, t + 1, len - 1);
        }
        while len > 0 && right.get(s + len - 1) == Some(&left[s + len - 1]) && left.get(t + len - 1) == Some(&right[t + len - 1]) {
            len -= 1;
        }
        if len >= MIN_MOVED_BLOCK && right.get(s..s + len) != Some(&left[s..s + len]) {
            out.push(MovedBlock { source: s..s + len, target: t..t + len });
            if out.len() == MAX_MOVED_BLOCKS {
                break;
            }
        }
    }
    out
}

/// Moved blocks: exact relocations, plus shifted runs found by aligning the changed region.
/// Sorted by source offset; at most `MAX_MOVED_BLOCKS`.
fn moved_blocks(left: &[u8], right: &[u8]) -> Vec<MovedBlock> {
    let mut out = relocated_blocks(left, right);
    let shift_hint = out.iter().map(|m| m.source.start.abs_diff(m.target.start)).filter(|&s| s <= WIDE_BAND).max().unwrap_or(0);
    add_aligned_runs(&mut out, aligned_runs(left, right, shift_hint));
    out.sort_unstable_by_key(|m| m.source.start);
    out
}

// Local alignment, after OxideMerge: a banded edit-distance alignment between the common
// prefix and suffix finds shifted runs (inserted or deleted bytes) that the unique-window
// search misses in repetitive data.
const ALIGN_BAND: usize = 32;
const ALIGN_WIDTH: usize = 2 * ALIGN_BAND + 1;
/// Traceback cells (one byte each) the alignment may use; past it, it is skipped.
const ALIGN_CELLS: usize = 4 * 1024 * 1024;
const WIDE_BAND: usize = 128;
const WIDE_WIDTH: usize = 2 * WIDE_BAND + 1;
/// Unchanged bytes kept around the changed region, to establish the alignment.
const ALIGN_CONTEXT: usize = ALIGN_BAND + MIN_MOVED_BLOCK;

/// Shifted runs `(left range, right range)` of equal bytes found by aligning the region
/// that differs. Empty when the region is too large for the cell budget.
fn aligned_runs(left: &[u8], right: &[u8], shift_hint: usize) -> Vec<(Range<usize>, Range<usize>)> {
    let prefix = common_prefix(left, right);
    let start = prefix.saturating_sub(ALIGN_CONTEXT);
    let suffix = common_suffix(&left[prefix..], &right[prefix..]).saturating_sub(ALIGN_CONTEXT);
    let (l, r) = (&left[start..left.len() - suffix], &right[start..right.len() - suffix]);
    let delta = l.len().abs_diff(r.len());
    let mut shift = if delta <= WIDE_BAND { shift_hint.max(delta) } else { shift_hint };
    if shift <= ALIGN_BAND {
        // A balanced insertion and deletion has no net shift: look for the bytes after the
        // first change a little further on, before choosing the narrow band.
        let at = common_prefix(l, r);
        let near = [(l, r), (r, l)]
            .into_iter()
            .filter_map(|(a, b)| {
                let needle = a.get(at..at + MIN_MOVED_BLOCK)?;
                let hay = b.get(at + 1..b.len().min(at + WIDE_BAND + MIN_MOVED_BLOCK))?;
                hay.windows(MIN_MOVED_BLOCK).position(|w| w == needle).map(|o| o + 1)
            })
            .min();
        shift = shift.max(near.unwrap_or(0));
    }
    let cells = |band: usize, width: usize| (l.len().min(r.len() + band) + 1).saturating_mul(width);
    let runs = if shift > ALIGN_BAND && cells(WIDE_BAND, WIDE_WIDTH) <= ALIGN_CELLS {
        align_window::<WIDE_BAND, WIDE_WIDTH>(l, r)
    } else if cells(ALIGN_BAND, ALIGN_WIDTH) <= ALIGN_CELLS {
        align_window::<ALIGN_BAND, ALIGN_WIDTH>(l, r)
    } else {
        return Vec::new();
    };
    runs.into_iter().map(|(a, b)| (a.start + start..a.end + start, b.start + start..b.end + start)).collect()
}

/// Banded edit-distance alignment with a traceback; returns the shifted runs of equal bytes
/// worth reporting (long, or supported by long neighbours).
fn align_window<const BAND: usize, const WIDTH: usize>(left: &[u8], right: &[u8]) -> Vec<(Range<usize>, Range<usize>)> {
    const INF: u32 = u32::MAX / 2;
    const DIAGONAL: u8 = 1;
    const DELETE: u8 = 2;
    const INSERT: u8 = 3;
    let (n, m) = (left.len(), right.len());
    let last_row = n.min(m + BAND);
    let mut trace = vec![0u8; (last_row + 1) * WIDTH];
    let mut prev = [INF; WIDTH];
    let mut endpoint = (u64::MAX, 0, 0);
    for i in 0..=last_row {
        let mut row = [INF; WIDTH];
        for j in i.saturating_sub(BAND)..=m.min(i + BAND) {
            let k = j + BAND - i;
            if i == 0 && j == 0 {
                row[k] = 0;
                continue;
            }
            // A substitution is preferred to a pair of gaps unless the bytes support a shift.
            let mut best = (INF, 0);
            if i > 0 && j > 0 {
                best = (prev[k] + 2 * u32::from(left[i - 1] != right[j - 1]), DIAGONAL);
            }
            if i > 0 && k + 1 < WIDTH && prev[k + 1] + 3 < best.0 {
                best = (prev[k + 1] + 3, DELETE);
            }
            if j > 0 && k > 0 && row[k - 1] + 3 < best.0 {
                best = (row[k - 1] + 3, INSERT);
            }
            row[k] = best.0;
            trace[i * WIDTH + k] = best.1;
            // When an end lies outside the band, finish at the shorter side's end and charge
            // for the remaining gap.
            if (n.abs_diff(m) > BAND && (i == n || j == m)) || (i == n && j == m) {
                let cost = u64::from(best.0) + 3 * (n - i + m - j) as u64;
                if cost <= endpoint.0 {
                    endpoint = (cost, i, j);
                }
            }
        }
        prev = row;
    }
    let mut runs: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    let (_, mut i, mut j) = endpoint;
    while i != 0 || j != 0 {
        match trace[i * WIDTH + j + BAND - i] {
            DIAGONAL if left[i - 1] == right[j - 1] => {
                let (end_i, end_j) = (i, j);
                while i > 0 && j > 0 && left[i - 1] == right[j - 1] && trace[i * WIDTH + j + BAND - i] == DIAGONAL {
                    i -= 1;
                    j -= 1;
                }
                runs.push((i..end_i, j..end_j));
            }
            DIAGONAL => {
                i -= 1;
                j -= 1;
            }
            DELETE => i -= 1,
            INSERT => j -= 1,
            _ => return Vec::new(),
        }
    }
    runs.reverse();
    // Short runs count only next to long ones; unshifted runs are context, not moves.
    let nearby = |a: &(Range<usize>, Range<usize>), b: &(Range<usize>, Range<usize>)| (b.0.start - a.0.end) + (b.1.start - a.1.end) <= 2;
    runs.iter()
        .enumerate()
        .filter(|(index, (a, b))| {
            if a.start == b.start {
                return false;
            }
            let current = &runs[*index];
            let prev = index.checked_sub(1).and_then(|i| runs.get(i));
            let next = runs.get(index + 1);
            let supported = a.len() >= 8 && (prev.is_some_and(|p| p.0.len() >= 8 && nearby(p, current)) || next.is_some_and(|n| n.0.len() >= 8 && nearby(current, n)));
            let short_header = a.len() >= 4 && a.start.min(b.start) == 0 && next.is_some_and(|n| n.0.len() >= MIN_MOVED_BLOCK && n.0.start == a.end + 1 && n.1.start == b.end + 1);
            (a.len() >= MIN_MOVED_BLOCK || supported || short_header) && right.get(a.clone()) != Some(&left[a.clone()]) && left.get(b.clone()) != Some(&right[b.clone()])
        })
        .map(|(_, pair)| pair.clone())
        .collect()
}

/// Adds the aligned runs to the relocated blocks, leaving out the parts a relocated block
/// already covers on either side.
fn add_aligned_runs(out: &mut Vec<MovedBlock>, runs: Vec<(Range<usize>, Range<usize>)>) {
    let found = out.len();
    for (source, target) in runs {
        let mut conflicts = Vec::new();
        for m in &out[..found] {
            for (run, used) in [(&source, &m.source), (&target, &m.target)] {
                let start = used.start.saturating_sub(run.start);
                let end = used.end.min(run.end).saturating_sub(run.start);
                if start < end {
                    conflicts.push(start..end);
                }
            }
        }
        conflicts.sort_unstable_by_key(|r| r.start);
        let minimum = if conflicts.is_empty() { 1 } else { MIN_MOVED_BLOCK };
        let (mut start, len) = (0, source.len());
        for c in conflicts.into_iter().chain(std::iter::once(len..len)) {
            if out.len() == MAX_MOVED_BLOCKS {
                return;
            }
            if c.start.saturating_sub(start) >= minimum {
                out.push(MovedBlock { source: source.start + start..source.start + c.start, target: target.start + start..target.start + c.start });
            }
            start = start.max(c.end);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn pieces<'a>(s: &'a str, r: &[Range<usize>]) -> Vec<&'a str> {
        r.iter().map(|r| &s[r.clone()]).collect()
    }

    #[test]
    fn inline_diff_marks_the_changed_words() {
        let (a, b) = ("Built on XPG 2026-06-13 00:39:14 PDT", "Built on XPG 2026-05-22 03:21:24 PDT");
        let (ra, rb) = inline_diff(a, b);
        assert_eq!(pieces(a, &ra), vec!["06", "13", "00", "39", "14"]);
        assert_eq!(pieces(b, &rb), vec!["05", "22", "03", "21", "24"]);
        let (a, b) = ("XPG Release RJ-2026.6", "XPG Release RJ-2025.5");
        let (ra, rb) = inline_diff(a, b);
        assert_eq!(pieces(a, &ra), vec!["2026", "6"]);
        assert_eq!(pieces(b, &rb), vec!["2025", "5"]);
    }

    #[test]
    fn inline_diff_handles_cjk_insertions_and_long_lines() {
        let (a, b) = ("設定名稱 abc", "設定的名稱 abc def");
        let (ra, rb) = inline_diff(a, b);
        assert!(ra.is_empty());
        assert_eq!(pieces(b, &rb), vec!["的", " def"]);
        assert_eq!(words("a_1  b,c"), vec![0..3, 3..5, 5..6, 6..7, 7..8]);
        let long = "x".repeat(MAX_INLINE_BYTES + 1);
        assert_eq!(inline_diff(&long, "y"), (vec![0..long.len()], vec![0..1]));
        assert_eq!(inline_diff("", "a"), (vec![], vec![0..1]));

        // Long, mostly different lines stay within the small word budget: the differing middle
        // (between the shared start and end) is highlighted as one piece.
        let a: String = (0..700).map(|i| format!("a{i} ")).collect::<String>() + "end";
        let b: String = (0..700).map(|i| format!("b{i} ")).collect::<String>() + "end";
        let start = std::time::Instant::now();
        let (ra, rb) = inline_diff(&a, &b);
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(ra, vec![0..a.len() - 4]);
        assert_eq!(rb, vec![0..b.len() - 4]);
    }

    fn v(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    /// Rebuilds both sides from the rows, proving every line appears once and in order.
    fn sides(d: &TextDiff) -> (Vec<usize>, Vec<usize>) {
        (d.rows.iter().filter_map(|r| r.left).collect(), d.rows.iter().filter_map(|r| r.right).collect())
    }

    #[test]
    fn identical_text_has_no_hunks() {
        let a = v(&["a", "b", "c"]);
        let d = diff_lines(&a, &a);
        assert!(d.is_identical());
        assert_eq!(d.rows.len(), 3);
        assert!(d.rows.iter().all(|r| r.kind == RowKind::Same));
    }

    #[test]
    fn changes_insertions_and_deletions_are_aligned() {
        let a = v(&["one", "two", "three", "four"]);
        let b = v(&["one", "TWO", "three", "four", "five"]);
        let d = diff_lines(&a, &b);
        assert_eq!(
            d.rows,
            vec![
                Row::new(Some(0), Some(0), RowKind::Same),
                Row::new(Some(1), Some(1), RowKind::Changed),
                Row::new(Some(2), Some(2), RowKind::Same),
                Row::new(Some(3), Some(3), RowKind::Same),
                Row::new(None, Some(4), RowKind::Inserted),
            ]
        );
        assert_eq!(d.hunks, vec![1..2, 4..5]);
        assert_eq!(d.stats(), TextStats { changed: 1, deleted: 0, inserted: 1, moved: 0 });
        assert_eq!(d.row_of(true, 4), Some(4));
        assert_eq!(d.row_of(false, 3), Some(3));
        assert_eq!(d.line_bounds(false), vec![0, 1, 2, 3, 4, 4]);
        assert_eq!(d.line_bounds(true), vec![0, 1, 2, 3, 4, 5]);

        let d = diff_lines(&b, &a);
        assert_eq!(d.rows[4], Row::new(Some(4), None, RowKind::Deleted));
    }

    #[test]
    fn options_ignore_blank_lines_case_and_line_endings() {
        let opts = |blank, case, eol| TextOptions { ignore_blank_lines: blank, ignore_case: case, ignore_eol: eol };
        // Blank lines added or removed on their own are not differences when ignored.
        let a = v(&["one", "", "two", "three"]);
        let b = v(&["one", "two", "", "", "three"]);
        let d = diff_lines_with(&a, &b, [None, None], opts(false, false, true));
        assert_eq!(d.hunks.len(), 2);
        let d = diff_lines_with(&a, &b, [None, None], opts(true, false, true));
        assert!(d.is_identical(), "only blank lines changed");
        assert!(d.rows.iter().filter(|r| r.kind != RowKind::Same).all(|r| r.ignored));
        assert_eq!(d.stats(), TextStats::default());
        // A change mixing blank and other lines still counts.
        let d = diff_lines_with(&v(&["x", ""]), &v(&["y"]), [None, None], opts(true, false, true));
        assert_eq!(d.hunks.len(), 1);

        // Case.
        let (a, b) = (v(&["Hello", "World"]), v(&["hello", "WORLD"]));
        assert_eq!(diff_lines_with(&a, &b, [None, None], opts(false, false, true)).hunks.len(), 1);
        assert!(diff_lines_with(&a, &b, [None, None], opts(false, true, true)).is_identical());

        // Line endings: the same text with CRLF on one side and LF on the other.
        let a = v(&["a", "b", ""]);
        let ea = line_endings("a\r\nb\r\n");
        let eb = line_endings("a\nb\n");
        assert_eq!(ea, vec![Eol::CrLf, Eol::CrLf, Eol::None]);
        assert_eq!(line_endings("x\ry"), vec![Eol::Cr, Eol::None]);
        assert!(diff_lines_with(&a, &a, [Some(&ea), Some(&eb)], opts(false, false, true)).is_identical());
        let d = diff_lines_with(&a, &a, [Some(&ea), Some(&eb)], opts(false, false, false));
        assert_eq!(d.hunks, vec![0..2], "both terminated lines differ; the last (none) does not");
        assert_eq!(d.rows[..2], [Row::new(Some(0), Some(0), RowKind::Changed), Row::new(Some(1), Some(1), RowKind::Changed)], "lines still pair up by text");
        // Without terminators for a side, styles are not compared.
        assert!(diff_lines_with(&a, &a, [Some(&ea), None], opts(false, false, false)).is_identical());
    }

    #[test]
    fn empty_sides() {
        let a = v(&["x", "y"]);
        let d = diff_lines(&a, &[]);
        assert_eq!(d.rows, vec![Row::new(Some(0), None, RowKind::Deleted), Row::new(Some(1), None, RowKind::Deleted)]);
        assert_eq!(d.hunks, vec![0..2]);
        assert!(diff_lines(&[], &[]).is_identical());
    }

    #[test]
    fn detects_moved_lines() {
        let a = v(&["moved", "a", "b", "c", "x"]);
        let b = v(&["y", "a", "b", "c", "moved"]);
        let d = diff_lines(&a, &b);
        assert_eq!(d.hunks.len(), 2);
        let first = d.rows[d.hunks[0].start];
        assert_eq!(first.left, Some(0));
        assert_eq!(first.left_moved, Some(4), "left line 0 moved to right line 4");
        assert_eq!(first.right_moved, None, "\"y\" is not in the left file");
        let last = d.rows[d.hunks[1].start];
        assert_eq!(last.right, Some(4));
        assert_eq!(last.right_moved, Some(0));
        assert_eq!(last.left_moved, None);
        assert_eq!(d.stats().moved, 2);
    }

    #[test]
    fn moved_block_of_several_lines_and_blank_lines_are_not_moved() {
        let a = v(&["fn a() {", "  body", "}", "", "keep 1", "keep 2", "keep 3"]);
        let b = v(&["keep 1", "keep 2", "keep 3", "", "fn a() {", "  body", "}"]);
        let d = diff_lines(&a, &b);
        let moved_left: Vec<_> = d.rows.iter().filter_map(|r| r.left.zip(r.left_moved)).collect();
        assert_eq!(moved_left, vec![(0, 4), (1, 5), (2, 6)]);
        let moved_right: Vec<_> = d.rows.iter().filter_map(|r| r.right.zip(r.right_moved)).collect();
        assert_eq!(moved_right, vec![(4, 0), (5, 1), (6, 2)]);
        assert!(d.rows.iter().all(|r| r.left != Some(3) || r.left_moved.is_none()));
    }

    #[test]
    fn swapped_lines_read_as_one_move() {
        let a = v(&["start", "p", "q", "end"]);
        let b = v(&["start", "q", "p", "end"]);
        let d = diff_lines(&a, &b);
        assert_eq!(
            d.rows.iter().map(|r| (r.left, r.right, r.kind)).collect::<Vec<_>>(),
            vec![
                (Some(0), Some(0), RowKind::Same),
                (Some(1), None, RowKind::Deleted),
                (Some(2), Some(1), RowKind::Same),
                (None, Some(2), RowKind::Inserted),
                (Some(3), Some(3), RowKind::Same),
            ]
        );
        assert_eq!(d.rows[1].left_moved, Some(2));
        assert_eq!(d.rows[3].right_moved, Some(1));
    }

    #[test]
    fn reordering_inside_one_change_is_not_a_move() {
        // No line is unique, so Myers aligns the middle as a single change holding "p" on
        // both sides: that is not a move.
        let a = v(&["s", "p", "p", "x", "e"]);
        let b = v(&["s", "y", "p", "p", "e"]);
        let d = diff_lines(&a, &b);
        assert!(!d.is_identical());
        for r in &d.rows {
            if r.left.is_some_and(|l| a[l] == "p") {
                assert_eq!(r.left_moved, None);
            }
        }
    }

    #[test]
    fn rows_keep_every_line_once_in_order_for_repetitive_input() {
        let a: Vec<String> = (0..400).map(|i| format!("{}", i % 7)).collect();
        let b: Vec<String> = (0..380).map(|i| format!("{}", (i * 3) % 5)).collect();
        let d = diff_lines(&a, &b);
        let (l, r) = sides(&d);
        assert_eq!(l, (0..a.len()).collect::<Vec<_>>());
        assert_eq!(r, (0..b.len()).collect::<Vec<_>>());
        for row in &d.rows {
            if row.kind == RowKind::Same {
                assert_eq!(a[row.left.unwrap()], b[row.right.unwrap()]);
            }
        }
    }

    #[test]
    fn many_unrelated_lines_fall_back_to_one_replacement() {
        let a: Vec<String> = (0..2000).map(|i| format!("a{}", i % 3)).collect();
        let b: Vec<String> = (0..2000).map(|i| format!("b{}", i % 3)).collect();
        let d = diff_lines(&a, &b);
        assert_eq!(d.hunks, vec![0..2000]);
        assert!(d.rows.iter().all(|r| r.kind == RowKind::Changed));
    }

    #[test]
    fn myers_finds_minimal_script() {
        let ops = myers(&[1, 2, 3, 4], &[1, 3, 4, 5], MAX_EDITS).unwrap();
        assert_eq!(ops, vec![Op::Equal(0, 0), Op::Delete(1), Op::Equal(2, 1), Op::Equal(3, 2), Op::Insert(3)]);
    }

    fn bytes(seed: u32, len: usize) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    #[test]
    fn binary_counts_differences_and_length_tail() {
        let a = b"hello world".to_vec();
        let b = b"hellO world!!".to_vec();
        let d = diff_bytes(&a, &b);
        assert_eq!(d.differing, 3);
        assert!(!d.is_identical());
        assert!(diff_bytes(&a, &a).is_identical());
        assert_eq!(d.next_run(&a, &b, 0), Some(4..5));
        assert_eq!(d.next_run(&a, &b, 5), Some(11..13));
        assert_eq!(d.next_run(&a, &b, 13), None);
        assert_eq!(d.prev_run(&a, &b, 13), Some(11..13));
        assert_eq!(d.prev_run(&a, &b, 11), Some(4..5));
        assert_eq!(d.prev_run(&a, &b, 4), None);
        assert!(d.cell_differs(&a, &b, 12) && !d.cell_differs(&a, &b, 0));
        assert_eq!(d.cell(12), [None, Some(12)], "the shorter side shows blank cells");
    }

    #[test]
    fn binary_runs_skip_long_equal_stretches() {
        let a = vec![0u8; 20_000];
        let mut b = a.clone();
        b[9_000] = 1;
        b[19_999] = 2;
        let d = diff_bytes(&a, &b);
        assert_eq!(d.next_run(&a, &b, 0), Some(9_000..9_001));
        assert_eq!(d.next_run(&a, &b, 9_001), Some(19_999..20_000));
        assert_eq!(d.prev_run(&a, &b, 19_999), Some(9_000..9_001));
    }

    #[test]
    fn binary_detects_a_moved_block() {
        // [x][block][y] -> [y'][x][block] with the block shifted right by 64 bytes.
        let x = bytes(1, 200);
        let block = bytes(2, 100);
        let y = bytes(3, 64);
        let a = [x.clone(), block.clone(), y.clone()].concat();
        let b = [y, x, block].concat();
        let d = diff_bytes(&a, &b);
        assert!(!d.moved.is_empty());
        // x moved from 0 to 64, and the block with it.
        assert_eq!(d.moved[0], MovedBlock { source: 0..300, target: 64..364 });
        for m in &d.moved {
            assert_eq!(a[m.source.clone()], b[m.target.clone()]);
            assert!(m.source.len() >= MIN_MOVED_BLOCK);
        }
        // The layout lines the moved block up: the right's first 64 bytes face blank cells.
        assert_eq!(d.stretches[0], Stretch { at: 0, len: 64, start: [0, 0], count: [0, 64], matched: false });
        assert_eq!(d.stretches[1], Stretch { at: 64, len: 300, start: [0, 64], count: [300, 300], matched: true });
        assert_eq!(d.cell(64), [Some(0), Some(64)]);
        assert_eq!(d.differing, 64 + 64, "64 inserted bytes, and y's 64 bytes deleted at the end");
        assert_eq!(d.next_run(&a, &b, 0), Some(0..64));
        assert_eq!(d.next_run(&a, &b, 64), Some(364..428));
        assert_eq!(d.prev_run(&a, &b, 364), Some(0..64));
    }

    #[test]
    fn binary_layout_lines_up_small_insertions_and_deletions() {
        // As in the sample firmware: 2 bytes deleted, 5 changed, 3 inserted, between long runs.
        let p = bytes(11, 60);
        let q = bytes(12, 80);
        let r = bytes(13, 90);
        let t = bytes(14, 70);
        let a = [p.clone(), vec![0xAA, 0xBB], q.clone(), vec![1, 2, 3, 4, 5], r.clone(), t.clone()].concat();
        let b = [p, q, vec![9, 8, 7, 6, 5], r, vec![0xCC, 0xDD, 0xEE], t].concat();
        let d = diff_bytes(&a, &b);
        // After the deletion, rows realign: q's first byte faces q's first byte.
        let at = |side: usize, off: usize| (0..d.cells()).find(|&c| d.cell(c)[side] == Some(off)).unwrap();
        assert_eq!(d.cell(at(0, 62)), [Some(62), Some(60)]);
        assert_eq!(d.cell(at(0, 60)), [Some(60), None]);
        assert_eq!(d.cell(at(1, 235)), [None, Some(235)], "inserted bytes face blank cells");
        assert_eq!(d.cell(at(0, 237)), [Some(237), Some(238)], "t lines up again");
        assert_eq!(d.differing, 2 + 4 + 3, "deleted, changed (5 and 5 match) and inserted bytes");
    }

    #[test]
    fn binary_ignores_short_and_repeated_matches() {
        // Repeated data and in-place edits are not moves.
        let a = vec![7u8; 4096];
        let mut b = a.clone();
        b[100] = 0;
        assert!(diff_bytes(&a, &b).moved.is_empty());
        let a = bytes(5, 1000);
        let mut b = a.clone();
        b[500] ^= 0xff;
        assert!(diff_bytes(&a, &b).moved.is_empty());
        assert!(diff_bytes(b"short", b"other").moved.is_empty());
    }
}
