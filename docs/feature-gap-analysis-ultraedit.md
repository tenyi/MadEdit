# MadEdit Scope Analysis

> Rewritten: 2026-11-05 (was "Feature Gap: MadEdit vs. UltraEdit")
> MadEdit baseline: `main` at commit `40a7328` (egui rewrite, v0.6.8)
> UltraEdit baseline: IDM UltraEdit v2026.x (page last modified 2026-10-29), including the
> tools IDM bundles with it (UltraCompare Professional, UltraGit, UltraFTP).

## What this document is

This is a **scope statement**, not a backlog. It records which UltraEdit capabilities MadEdit
adopts, which ones it defers, and which ones it deliberately does not implement.

MadEdit is a **fast plain-text / column / hex editor** with strong multi-encoding support.
It is not an IDE, and it does not try to become one. The original MadEdit was a text, column
and hex editor with an unusual number of encodings; that is the product, and everything else
serves it.

## The decision rule

A capability is considered only when it answers one question:

> **Does this make editing plain text faster or safer?**

If a capability mainly serves software development workflows — understanding a project,
running tools, integrating with version control or remote hosts — it is out of scope even
when a commercial editor ships it. If a capability mainly serves reading and writing text
files — including log files, configuration files, data dumps and legacy-encoded documents —
it is in scope.

"UltraEdit has it" is never a reason on its own. The comparison is used to catch the case
where a plain-text editor is expected to do something basic and MadEdit cannot, not to
enumerate UltraEdit's feature surface.

## Decision summary

| UltraEdit capability | Decision | Rationale |
| --- | --- | --- |
| Multi-caret / multi-select | **Adopt** | A basic expectation of any text editor; affects everyday editing, not project tooling |
| Code folding | **Adopt** | Cheap with the existing syntax definitions, and useful for long configuration and log files |
| File and folder compare / merge | **Defer** | Genuinely useful for text work, but the diff engine plus three-way merge plus UI is a large project. Until it is built, point users at an external diff tool |
| Search in selection, file-type filters, recursive search, search in open files | **Already at parity** | Implemented in the Find bar and the Find in Files panel |
| Column / block editing, hex editing, multi-encoding handling, bookmarks, macros, spelling, printing, session restore, print preview | **Already at parity** | Part of the original MadEdit command set |
| IntelliTips — context-aware code completion | **Out of scope** | IDE territory: it serves code authoring, not text editing, and implies a language model of the project |
| Integrated scripting language (JavaScript-like), Smart Templates | **Out of scope** | An automation platform is a separate product with its own security and maintenance cost |
| Git integration | **Out of scope** | Version control is a dedicated tool; MadEdit edits files, not repositories |
| FTP / SFTP / FTPS, SSH / Telnet console | **Out of scope** | Remote host management is a separate product, and it is not cross-platform (Windows-only) |
| XML Manager tree view, XML lint, HTML Tidy, tag auto-completion | **Out of scope** | Web-development tooling. MadEdit formats XML and JSON and renders Markdown; it does not model documents |
| Built-in HTML live preview with source navigation | **Out of scope** | The in-app Markdown preview already covers the preview need, without embedding a browser engine |
| Data format conversion (CSV ↔ JSON ↔ XML) | **Out of scope** | Data conversion belongs in dedicated tools |
| File explorer pane, project files, Favorites lists | **Out of scope** | Workspace management is an IDE concern. Tabs, recent files, session restore and Find in Files cover the text-editing workflow |
| Customizable menus and toolbars | **Out of scope** | A fixed, familiar command set is part of the product's simplicity. Every command's shortcut can already be rebound |
| File type association, shell extensions, e-mail sending | **Out of scope** | Operating-system integration, not editing. The menu item already prints the steps to register the app manually |
| Binary structure templates, hex file merge, ASCII/Unicode character tables | **Out of scope for now** | The hex view stays a plain offset / bytes / ASCII browser with byte search |
| Symmetrical editing, more date/time formats, more versioned backups | **Out of scope** | Marginal value for the target workflow; a single `.bak` copy and one configurable date format are enough |

## Deliberately out of scope

The following are implemented by commercial editors of this class and are **intentionally
absent** from MadEdit. A feature request for any of them should be declined with a pointer
to this section rather than scheduled.

1. **Language intelligence** — code completion, function tips, symbol navigation, language
   servers. MadEdit highlights text; it does not understand the project.
2. **Automation platforms** — a scripting runtime, macro languages with control flow,
   user-defined commands bound to toolbars. MadEdit's macros record and replay a step list;
   they do not branch.
3. **Repository and remote-host integration** — Git, FTP, SFTP, SSH, Telnet.
4. **Workspace and project management** — file explorer panes, project files, workspace
   models, multi-root views.
5. **Document modelling toolchains** — XML tree editing, HTML tidy and validation, schema
   awareness, data format conversion.
6. **Comparison and merging as a platform** — folder comparison and synchronisation, three-way
   merge across arbitrary sources.
7. **Embedded browser and rich rendering** — MadEdit previews Markdown with Mermaid diagrams
   and LaTeX math in-process. It does not embed a browser engine, and it does not preview
   HTML inside the window.

What MadEdit offers instead of each of these is deliberately narrow: the file is opened, read
and written correctly, and nothing else is required of the user.

## Already at parity with UltraEdit

For the record, these UltraEdit capabilities MadEdit already implements, so they need no
discussion:

- Column / block mode with visual-column (CJK-aware) rectangular selection, aligned paste and
  simultaneous typing; hex mode with byte search and jump-to-offset.
- Multi-encoding support with BOM detection, a configurable fallback encoding, and refusal to
  overwrite bytes that cannot be decoded losslessly.
- Find and replace (plain, case, whole word, regex, hex), Find in Files with recursive
  subfolders, wildcard include/exclude filters, "opened files only" and search in selection.
- Bookmarks, brace matching, sorting, number insertion, case and full/half width conversion,
  comment toggling, tab/space conversion.
- Spelling (Hunspell) with suggestions and a personal dictionary; macros with record, play,
  save and load; word count.
- Printing with page setup, headers and footers, and an in-app print preview.
- Multi-document tabs, session restore, caret position memory, single-instance forwarding.

## Where MadEdit deliberately differs

MadEdit is not trying to match UltraEdit feature for feature. These are places where
MadEdit's approach is intentionally its own:

1. **Lossless encoding handling.** UTF-8, UTF-8 BOM, UTF-16LE/BE, Big5, GB18030, Shift_JIS,
   EUC-KR, windows-1252, ISO-8859-1 and more, with BOM detection, a configurable fallback
   encoding, and a refusal to overwrite a file whose bytes cannot be decoded losslessly.
   UltraEdit has no equivalent safeguard.
2. **CJK column mode.** Rectangular selection is measured in visual columns (1 for
   half-width, 2 for full-width CJK, tab-aligned). UltraEdit's column mode is weaker for
   CJK text.
3. **Built-in Markdown preview with Mermaid diagrams and LaTeX math**, rendered in pure
   Rust (`fermaid`, `latex-rust`), with scroll sync and a zoomable split view. UltraEdit's
   Markdown preview has neither.
4. **Offline dprint formatting** for more than 20 languages, bundled in the `-dprint` release
   flavour.
5. **Cross-platform parity.** Windows, macOS and Linux share one Rust/egui code base with
   native IME, system font selection and single-instance forwarding.
6. **Free and open source**, plus phrase-based Simplified/Traditional conversion that
   resolves ambiguous characters (头发→頭髮 vs 开发→開發).

## Performance guardrails

"Fast" is the product claim, so it has to be measurable rather than a slogan. The metrics
that decide whether a feature belongs in MadEdit:

| Metric | Where it is measured |
| --- | --- |
| Cold start to an editable window | `make bench` |
| Time to first painted frame of a large file | `make bench`, ADR 0012 |
| Scrolling and search latency in a 100 MB file | `make bench` |
| Time to open a 1 GiB file | `make test-large` |
| Long-line syntax highlighting cost | ADR 0012 (time-sliced parsing is intentional) |

Rules of thumb:

- A new feature must not add work to the interactive path that scales with the whole
  document. Visible-window rendering, time-sliced syntax parsing and content-hash-based
  caches exist for this reason and must not be bypassed.
- Any change that measurably regresses the metrics above needs a justification in the pull
  request, not silence.
- Whole-document copies or scans in an interactive path are rejected outright.
- Adopting a feature that only pays off on multi-gigabyte inputs (for example full-file
  indexing) is out of scope; MadEdit keeps files on disk and reads them on demand.

See [testing.md](testing.md) for the test and benchmark procedures.

## Notes on the UltraEdit reference

- The UltraEdit feature set differs between its Windows, macOS and Linux builds, and some
  capabilities ship as separately licensed tools (UltraCompare Professional, UltraGit,
  UltraFTP) rather than as editor features.
- UltraEdit's documentation was not readable end-to-end while this document was written.
  The reference material is the official feature list
  ([ultraedit.com/products/ultraedit/features/](https://www.ultraedit.com/products/ultraedit/features/)),
  the product overview ([ultraedit.com/products/](https://www.ultraedit.com/products/))
  and [Wikipedia](https://en.wikipedia.org/wiki/UltraEdit).
- Binary structure templates and hex file merge appear only in secondary descriptions of
  UltraEdit's hex editor; they are listed as out of scope, so no decision depends on them.

## Maintaining this document

- When a capability moves from *Defer* to *Adopt*, implement it, then update the row and the
  parity section in the same commit.
- When a capability is added to *Deliberately out of scope*, state the reason in one line.
  Requests that match this list are declined, not scheduled.
- The MadEdit-side statements are checked against the source tree: the command table
  (`src/app/commands.rs`), the implementations (`src/app/actions.rs`), the menus
  (`src/app/menus.rs`), the core modules (`src/core/`) and the settings schema
  (`src/core/settings.rs`). Re-verify them when the command table changes.
