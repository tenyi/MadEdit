# MadEdit Agent Guide

Repository-wide instructions for coding agents. MadEdit is a Rust text, column/block and hex editor for Windows, macOS and Linux. Keep changes small, preserve user data and follow existing patterns.

## Working agreement

- Work on a new `feat/<name>` branch based on the current `main`, not on `main` itself. Preserve unrelated changes and untracked files.
- Push the working branch to the contributor's fork and open a PR against `tenyi/MadEdit:main`. Do not push directly to upstream, merge a PR or create release tags unless explicitly requested. Check remote URLs before pushing; remote names alone are not authority.
- Write this guide, new documentation, code comments, commit messages and PR titles/descriptions in **English**. Use Conventional Commits (`feat:`, `fix:`, `docs:`, `refactor:`, etc.). Reply to the maintainer in the language they use.
- Leave existing Traditional Chinese comments alone unless changing the surrounding code. Runtime UI strings follow the localization rules below, not the documentation language rule.
- Follow KISS: keep UI-independent logic in `src/core/` and egui/platform interaction in `src/app/` or the platform modules. Avoid unnecessary abstractions, dependencies and whole-document copies or scans in interactive paths.
- Read the relevant implementation and nearby tests before editing. Current code, `Cargo.toml` and `Cargo.lock` define current behavior; dated ADRs and `.scratch/` specs may describe superseded designs.

## Project map

| Area | Start here |
| --- | --- |
| Startup, native open-file events, single instance | [`src/main.rs`](src/main.rs), [`src/macos.rs`](src/macos.rs), [`src/single_instance.rs`](src/single_instance.rs) |
| App state, tabs, save/close/quit, settings persistence | [`src/app/mod.rs`](src/app/mod.rs) |
| Commands, shortcuts, dispatch, menus and toolbar | [`commands.rs`](src/app/commands.rs), [`actions.rs`](src/app/actions.rs), [`menus.rs`](src/app/menus.rs) |
| Text view: shared state, layout, paint, input | [`src/app/view/`](src/app/view/); keep the `view::*` public API stable during internal refactors |
| Hex view, find/replace, multi-file search | [`hexview.rs`](src/app/hexview.rs), [`find.rs`](src/app/find.rs), [`findfiles.rs`](src/app/findfiles.rs) |
| Dialogs, form widgets, localization and fonts | [`dialogs.rs`](src/app/dialogs.rs), [`form.rs`](src/app/form.rs), [`i18n.rs`](src/app/i18n.rs), [`fonts.rs`](src/app/fonts.rs) |
| Markdown/Mermaid/math; print model and preview | [`mdpreview.rs`](src/app/mdpreview.rs), [`mdmath.rs`](src/app/mdmath.rs), [`print.rs`](src/app/print.rs), [`preview.rs`](src/app/preview.rs) |
| Document I/O, encoding, undo/redo, text/hex storage | [`document.rs`](src/core/document.rs), [`editor.rs`](src/core/editor.rs), [`encoding.rs`](src/core/encoding.rs), [`piece_table.rs`](src/core/piece_table.rs), [`hexbuf.rs`](src/core/hexbuf.rs) |
| Search, syntax caches, settings, formatting | [`search.rs`](src/core/search.rs), [`syntax.rs`](src/core/syntax.rs), [`settings.rs`](src/core/settings.rs), [`dprint.rs`](src/core/dprint.rs), [`textops.rs`](src/core/textops.rs) |
| Regression tests and fixtures | Tests beside the Rust code, [`ui_tests.rs`](src/app/ui_tests.rs), [`visual_tests.rs`](src/app/visual_tests.rs), [`tests/`](tests/), [`src/test_support.rs`](src/test_support.rs) |
| Build, packaging and releases | [`Makefile`](Makefile), [`scripts/`](scripts/), [release workflow](.github/workflows/release.yml) |

Read [README.md](README.md) for features, platform setup and packaging; [docs/testing.md](docs/testing.md) for test/benchmark procedures; [ADR 0011](docs/adr/0011-egui-rewrite.md) for the egui rewrite and [ADR 0012](docs/adr/0012-long-line-syntax-highlighting.md) for long-line performance tradeoffs. Earlier Slint UI and mmap designs are historical, not implementation instructions.

## Build, run and verify

Use Rust **1.99 or newer** (Edition 2024; see `rust-version` in `Cargo.toml`) and a C compiler for syntect's bundled Oniguruma backend. Keep `Cargo.lock` committed. macOS packaging also needs Xcode Command Line Tools. Run commands from the repository root.

| Task | Command | Notes |
| --- | --- | --- |
| Type check | `cargo check --locked` | `make check` runs `cargo check`, without `--locked` |
| Default Rust suite | `make test` | `cargo test --locked`; includes headless egui interaction tests |
| Core / app tests | `make test-core` / `make test-ui` | Filters `core::` / `app::` respectively |
| Focused regression | `cargo test --locked <test_name>` | Confirm the filter actually runs the intended test |
| Run the app | `cargo run --locked -- README.md` | Use an isolated `MADEDIT_CONFIG_DIR` for manual testing |
| Release build | `make build` | `cargo build --release --locked` |
| Visual comparisons | `make test-visual` | Ignored by default; needs a wgpu adapter and reviewed OS-specific baselines |
| Large-file streaming | `make test-large` | Ignored 1 GiB regression; allow at least 2 GiB of temporary space |
| Benchmark tooling / measurements | `make test-tools` / `make bench` | Python standard library; measurements run serially in release mode |
| Retained macOS AccessKit harness | `cargo test --manifest-path vendor/accesskit_macos/Cargo.toml --test integration` | Tests reference backport code, not the app's released dependency; see `vendor/README.md` |
| macOS packaging / native runtime | `./scripts/test_macos_bundle.sh` / `MADEDIT_CONFIG_DIR="$PWD/target/native-test-config" python3 scripts/test_macos_runtime.py` | Build the app first; runtime needs a logged-in desktop and checks PNG validity/exit status, not pixels |
| Release-note extraction | `make release-notes VERSION=Unreleased` | Replace `Unreleased` with the actual release version when needed |

`make` targets use a POSIX shell and Unix utilities. In native Windows PowerShell, use the Cargo equivalents; for packaging use `powershell -ExecutionPolicy Bypass -File .\scripts\make_windows_dist.ps1` (add `-Arch aarch64` for ARM). Do not copy POSIX environment-assignment syntax into PowerShell.

For an offscreen screenshot on macOS/Linux (system fonts, not a baseline comparison):

```bash
MADEDIT_SHOT_OUT=target/shot.png MADEDIT_SHOT_FILE=README.md cargo test --locked shot -- --ignored
```

The screenshot harness also accepts `MADEDIT_SHOT_CMDS=menuHexMode` and `MADEDIT_SHOT_CLICK=label1,label2` for exact widget labels. See `visual_tests.rs` before adding a scenario.

### Verification expectations

- Rust changes: add or update deterministic behavioral regression tests for changed core logic, run the focused tests, then `cargo check --locked` and `make test` with no new warnings. Cover error paths and exact content/selection/bytes, not just successful execution.
- UI changes: exercise the actual interaction with the existing egui harness and inspect an offscreen or native screenshot. Preserve IME, CJK/proportional-font geometry, keyboard focus, light/dark appearance and small-window behavior. Add new dialogs to `test_every_dialog_keeps_its_buttons_inside_its_window`.
- Native/macOS or packaging changes: run the relevant platform checks above. Headless rendering does not prove native window behavior, candidate-window positioning, Finder/Dock events or clean shutdown.
- Documentation-only changes: check paths, links, command accuracy and release-note extraction if affected; a Rust rebuild is not required when no code, dependency, build or test input changes.
- Tests must not touch everyday settings, sessions, clipboard or source files. Default test builds isolate settings and clipboard; use explicit temporary paths (see `src/test_support.rs`), not process-global environment mutation in parallel tests. Native/manual runs, **including `--screenshot`**, need a separate `MADEDIT_CONFIG_DIR`: screenshot mode uses default app settings and skips normal session saves, but startup still loads settings for window geometry.
- Never silently regenerate visual baselines or relax assertions to hide a failure. Review all changed images; record the platform, commands, results and any unavailable checks in the PR. Benchmark results exclude native window/GPU presentation; do not present them as end-to-end timings.

## Correctness and performance guardrails

### Document and storage

- Ordinary text uses decoded `EditorState` lines as its source of truth. Files above `OpenOptions::max_text_size` (default 100 MiB) are hex-only; do not decode or clone a GB-scale file to support a text-only action.
- File-backed Piece Table data uses cached **positioned reads**, not mmap, to avoid SIGBUS on external truncation. Current reads zero-fill unavailable bytes rather than reporting read errors. Keep hex access bounded and saves streamed through the pieces.
- Preserve encoding, BOM, newline format and original bytes for lossless reinterpretation. Reject unrepresentable output and conversion of decode-error text; lossy text saves require existing explicit user confirmation. Encoding/write failures must not truncate the target or discard the in-memory document.
- Keep external-change checks, atomic replacement and read-only protections on every affected save path. Unsaved-change confirmation applies to native quit as well as editor commands; cancelling quit must preserve all tabs and the session.
- Make text changes through the existing `Document` edit/undo/redo machinery. Declared changed ranges must be exact; keep syntax line identities/checkpoints, bookmarks, character-position anchors and view caches consistent after insert/delete/undo/redo. Whole-document resets clear incremental state.
- Distinguish UTF-8 byte offsets, character columns and visual columns. Snap byte windows to character boundaries; preserve CJK/emoji/tab and rectangular-selection behavior. Avoid duplicate whole-edit snapshots; do not drop raw recovery bytes for malformed, legacy or mixed-newline input before a real change is established.

### Rendering and dependencies

- Preserve visible-window, time-sliced syntax highlighting and content-hash-based layout/wrap/spelling caches. Do not reparse or measure an entire huge line every frame; long-line syntax is intentionally approximate, with foreground-only text outside the parsed window (ADR 0012).
- Keep measured glyph advances shared by painting, caret, selection, hit testing and wrapping. Font changes must invalidate width/layout caches after egui applies the fonts; syntax bold/italic must not change editor geometry.
- Keep egui, eframe, egui_extras and egui_kittest versions aligned. Match `fontique` to eframe's dependency and `resvg` to egui_extras' dependency; mismatches can break Linux fontconfig builds or SVG integration.
- Retain egui's `monochrome_emoji_fonts` feature. Keep egui_extras on `svg`, not `svg_text` (which loads all system fonts). Use latex-rust's SVG output, not its `egui` feature, which targets a different egui version.
- Reuse `mdpreview::DiagramLoader` for Mermaid/math rasterization. Supply actual palette colors to SVG, not CSS variables unsupported by resvg. Preserve CJK/Hangul fallback without loading every system font into the SVG font database.
- Preserve Markdown preview cache reuse and two-way scroll anchors. Resolve local images relative to the saved document, never the working directory for an unsaved file; HTTP images are not fetched. Reload external image changes only through the user's reload action.

## UI conventions

- Dispatch commands through `commands::C` and `App::run`; preserve command IDs persisted in settings/macros and keep the table grouped by menu for shortcut sections. Add View toggles to the appropriate submenu. Reuse the `menu`/`check_label` helpers for scrollable, aligned menus.
- Build settings with `form::section`/`card`, switches and segmented controls. Use `form::icon_button` with a translated action name, including the item name when relevant; a tooltip or symbol alone is not an accessible name. Use unique interaction IDs (`ui.unique_id()` for transient sibling rows), not shared scope IDs.
- Every dialog starts with `begin_dialog`; reuse `dialog_width`, `dialog_body`, `footer`, `primary_button` and `danger_button`. Keep footer actions reachable at small sizes. Fixed-height scroll areas need both minimum and maximum scrolled height; size separators inside horizontal rows to avoid growing auto-sized windows.
- Keep dialog focus ownership and restoration in `show_dialogs`. Use `enter_pressed` for safe primary actions; do not make destructive actions implicit Enter defaults. Preserve Escape exceptions for open popups, shortcut capture and confirmations. Apply actions mutating `self` after drawing closures.
- Preserve the Options draft while other commands run. Apply only edited fields with `Settings::with_edits` so live changes and recent history survive; save View preferences changed outside Options immediately through the existing persistence path.
- Localize UI text with `App::t`/`tf` and `assets/i18n/messages.tsv`; Traditional Chinese is the source text in code, not a TSV column. Keep translation keys unique and the TSV column structure intact. Background workers report typed outcomes, translated at paint time, not preformatted user-facing strings.
- Keep live-search regex and match counts cached by query/options/document version; caret movement must not rescan the document. Keep per-line highlights content-keyed, clear them when the bar closes, and do not enable replacement for read-only or hex documents. Cancel the previous Find in Files worker when starting another search.

## Current state and platform limits

- The UI is **egui/eframe**, with custom text/column and hex views; Slint is no longer a dependency. Single-instance forwarding **is implemented** in `src/single_instance.rs`, scoped by settings directory. Reopening a file selects its tab; distinct `MADEDIT_CONFIG_DIR` directories allow independent instances. Do not delete the instance lock file; drain forwarded requests in `App::logic` so minimized windows still respond.
- RTL is mirrored layout, not a full Unicode bidi implementation. Regex search is line-oriented. Macros are JSON step lists, not embedded Python. Check the current implementation before treating an old ADR's feature-gap list as current work.
- Keep the Finder/Dock Apple Event bridge in `src/macos.rs` without replacing winit's delegate, and preserve system shortcuts and uninterrupted IME composition. The app uses released AccessKit with the KVO/lifecycle fix; [`vendor/accesskit_macos`](vendor/accesskit_macos) is reference/test material, **not an active dependency override**. Follow [`vendor/README.md`](vendor/README.md) for provenance and removal conditions; do not disable accessibility to work around a crash.
- Public macOS packages use ad-hoc signing, not Developer ID/notarization. Packaging verifies signatures before replacing an installed app and supports custom Cargo output locations. [macOS validation](docs/macos-validation.md) records tested environments; one Apple Silicon pass does not prove Intel or all macOS versions.
- Reviewed visual baselines currently cover macOS only. The previous guide records pre-existing plain-text glyph baseline drift; do not treat it as a new regression or silently approve replacement images. Investigate differences against the relevant base revision when working on rendering.

## Documentation, assets and releases

- `README.md` is the English source of truth. Whenever it changes, update every translated README (currently `README_zh-TW.md`) in the same commit and keep section structures aligned. New translations use `README_<locale>.md` and must appear in every README's language list.
- Add release-relevant changes to `ChangeLog.md` under `[Unreleased]`. Keep English entries and the matching `### 繁體中文` translation in sync, using `#### 新增` / `#### 變更` / `#### 修正`. Release notes are extracted verbatim; only version boundaries may begin with `## [`.
- Do not hand-edit generated toolbar PNGs in `assets/icons/`. Update the XPM sources in `assets/icons-src/`, preserve their provenance/license and regenerate with `python3 assets/make_toolbar_icons.py`. Follow [`tests/fixtures/README.md`](tests/fixtures/README.md) for test-font sources and regeneration.
- `assets/formatters/manifest.json` is the authority for pinned dprint/plugin versions and SHA-256 values; keep config, language mapping, downloads and packaging in sync. POSIX formatter builds use `MADEDIT_WITH_FORMATTER=1`; Windows requires `-WithFormatter`. The release workflow requests clean and `-dprint` flavors, but current AppImage staging/output names lack the formatter suffix: verify both artifacts before releasing. The macOS helper's Wasm entitlement belongs to the helper, not the app binary.
- Packaging targets are `make dist-macos`, `make dist-linux` and `make dist-appimage` (`ARCH=x86_64` or `aarch64` for Linux); use the Windows script above on Windows. Linux cross-builds require cargo-zigbuild and Zig. Follow README for installation/release procedures rather than overwriting an installed app during routine checks.
- Keep this file concise and actionable: update durable conventions, architecture and limits when they change; put feature/release history in the changelog and design rationale in ADRs. Do not append completed phase lists, copied README sections or unqualified benchmark numbers. PRs should state scope, verification actually run and remaining platform limits.
