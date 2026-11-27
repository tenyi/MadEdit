# ADR 0013: One text-search core with two matching engines

- Status: accepted
- Date: 2026-11-27
- Related: [ADR 0004](0004-find-and-replace-engine.md) (its decision 1 is superseded by this one), [ADR 0011](0011-egui-rewrite.md)

## Context

ADR 0004 put the search engine in `src/core/search.rs`, apart from the UI. Two UI rewrites later the engine had drifted away from its callers:

- `core::search` still exposed its first API (`find_all_matches`, `find_next_match`, `replace_all_matches`, …) while `app::find` had grown its own matching and replacement code around `EditorState`. Only `build_regex` and `parse_hex_query` were shared; the rest of the core was dead behind `#![allow(dead_code)]`.
- A Unix / UltraEdit regular-expression dialect (`RegexFlavor`) had been added to the core but was unreachable: no setting, control or macro step selected it, and the app's replace paths bypassed its replacement translation.
- A pattern that crossed line breaks was matched by joining the whole document into a `String`, once per key press and again per Find Next, and only after the line-by-line pass had returned nothing. An alternation such as `a\nb|b` therefore never found its cross-line branch.
- The core reported failures as Traditional Chinese strings, which every interface language then showed.
- Replace All with "Selected text" checked fell back to the whole document when nothing was selected.

## Decision

1. `core::search` is the only text matching and replacement implementation. `app::find` owns the caches and the window and adds nothing to matching itself.
2. Two engines share one navigation contract: forward, the first match that starts at or after the caret; backward, the last match that ends at or before it; optional wrap; empty matches skipped. `find_in_lines` matches line by line and never copies the document. `find_in_spans` navigates byte ranges of matches found in `JoinedText`, the document joined with `\n`.
3. A pattern uses the joined engine when it contains a literal newline, or the `\n` escape as a regular expression, but not the literal text `\\n`. Other expressions that accept a line break (`\s`, `[^x]`) stay line-oriented, so everyday patterns do not copy the document.
4. `JoinedText` is cached by document id and version and rebuilt only after an edit. The match ranges come from the pass that counts them, so navigation and the "n / N" status never rescan.
5. Failures are a typed `SearchError`. The app translates it when it is shown.
6. The dialect code is removed. It can return as a follow-up that wires a setting, a control, macro steps and the replacement translation together.

## Consequences

- A cross-line query costs one copy of the document per edit while the find window is open, instead of one per key press plus one per Find Next.
- Replacing inside a stream selection and replacing in Find in Files use the same function as replacing the document.
- Cross-line matches are not highlighted in the editor, because highlighting is per line. The status row still counts and ranks them.
- "Selected text" needs a stream selection. Without one, Replace All does nothing and the button says why.
- A pattern such as `\s*` still matches within a line only, even though it could cross a line break.
