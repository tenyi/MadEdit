# MadEdit Project Guide (AGENTS.md)

This document is written for AI agents taking over the project. It gives the full project background, architecture, current progress and guidance for further development.

## Documentation Language

- Write all content in `ChangeLog.md` in English, including headings and release entries. GitHub Release notes are extracted directly from this file.
- Each released version section also carries a `### 繁體中文` block below the English entries, a faithful Traditional Chinese translation of the same items under `#### 新增` / `#### 變更` / `#### 修正`. Most users read the Chinese notes, so keep both in sync when a release section changes.
- The translation block must not contain a heading that starts with `## [`, because `scripts/changelog_notes.sh` ends a version section at that marker.

---

## 1. Background and Vision

- **Pain points of the original**: The C++ / wxWidgets MadEdit was hard to maintain on Linux and macOS: too many dependencies, a build chain that is hard to keep working, and frequent environment conflicts.
- **Goal of the rewrite**: Rebuild MadEdit in modern Rust as a lightweight, very fast, low-dependency professional text and hex editor that runs smoothly on Windows, macOS and Linux.
- **Core features (goals, not the current state; see section 5 for the current state)**:
  1. Text Mode
  2. Column / Block Mode
  3. Hex Mode (three columns: Offset, Hex, ASCII)
  4. Multi-encoding support (UTF-8, UTF-16, Big5, GB18030, Shift-JIS, etc.)
  5. Very large files (GB scale) with fast open and random access

---

## 2. Core Technology Choices

- **Language**: Rust (Edition 2024; latex-rust needs Rust 1.99 or newer, egui / eframe 0.37 needs 1.95)
- **GUI framework**: [eframe / egui](https://github.com/emilk/egui) (v0.37), rewritten from Slint in 2026-10 (see [ADR 0011](docs/adr/0011-egui-rewrite.md))
  - Why: it can measure width per character and supports any system font the user picks (proportional fonts included); input method events (`Event::Ime`) go straight to the custom-drawn editor.
  - The editor, hex view, scrollbars and ruler are all custom-drawn (`src/app/view.rs`, `src/app/hexview.rs`).
- **System fonts**: `fontique` enumerates and loads system fonts; CJK characters automatically fall back to a system CJK font.
- **Markdown preview**: `egui_commonmark` renders Markdown. `fermaid` (pure Rust) turns ```mermaid blocks into SVG, written with the actual colors from `Palette` (resvg does not support CSS variables), then rasterized through a custom loader (`mdpreview::DiagramLoader`; the resvg font database holds a single CJK font and does not load all system fonts) for display. `egui_extras` enables only `svg`, not `svg_text` (which would load all system fonts).
- **Markdown math**: `latex-rust` (pure Rust) lays out `$…$` / `$$…$$` into an SVG of STIX Two Math glyph outlines, drawn through the same `DiagramLoader` as Mermaid (`src/app/mdmath.rs`, hooked in with egui_commonmark's `render_math_fn`). Use only its SVG output: its `egui` feature pins egui 0.36.2. The math font has no CJK glyphs and the crate has no font fallback, so a top-level `\text{…}` with non-ASCII text is cut out and drawn as egui text; anything else it cannot render shows the TeX source.
- **Version alignment**: `fontique` must match the version eframe uses internally (currently 0.11.1); otherwise the fontconfig features conflict on Linux and the build fails.
- **Emoji glyphs**: egui 0.37 only bundles its emoji fonts behind the `monochrome_emoji_fonts` feature, so `egui` is listed directly in `Cargo.toml` just to enable it (keep its version equal to eframe's). Without it, 🔒 and other emoji draw as empty boxes.
- **System clipboard**: `arboard`; **syntax highlighting**: `syntect` with the Oniguruma regex backend (`regex-onig`; `onig_sys` builds the bundled C sources, so a C compiler is required); **spell check**: `spellbook` (Hunspell dictionaries).
- **File dialogs**: `rfd` (Rust File Dialog), with native cross-platform dialogs.
- **Encoding conversion**: `encoding_rs`, high-performance multi-encoding support.
- **Large file reads**: the Piece Table reads the original file in cached pages with positioned reads (pread). It does not use mmap (to avoid SIGBUS when a file is truncated externally).
- **App icon**: `embed-resource` (build-dependency), enabled only for Windows targets; it embeds `assets/madedit-icon.ico` into the `.exe` resources and is a no-op on other platforms.
- **Design philosophy**: **KISS (Keep It Simple, Stupid)**. Avoid over-abstraction and premature optimization; keep core data logic (`src/core/`) separate from the UI layer (`src/app/`).

---

## 3. Project Layout

```text
MadEdit/
├── Cargo.toml              # Dependencies and package settings
├── Cargo.lock              # Dependency lock file (must be committed)
├── Makefile                # Development, packaging, and extracting release notes from ChangeLog.md
├── ChangeLog.md            # Change log; GitHub Release notes come from the matching version section
├── README.md               # English README (source of truth for all READMEs)
├── README_zh-TW.md         # Traditional Chinese README (translation of README.md)
├── build.rs                # Windows icon resource embedding
├── .github/workflows/release.yml # Tag release (Linux, AppImage, macOS DMG, Windows)
├── .gitignore
├── assets\                 # Icon assets and desktop integration files
│   ├── madedit-icon.png    # Window and macOS icon (reused from the original madedit-mod.png, embedded with include_bytes)
│   ├── madedit-icon.ico    # Windows executable icon resource (reused from the original images/Mad2.ico)
│   ├── madedit.desktop     # Linux desktop entry
│   ├── icons-src\          # Original MadEdit toolbar icon sources (XPM, do not edit by hand)
│   │   └── README.md       # Icon sources and GNU GPL license notes
│   ├── icons\              # PNGs converted from icons-src (do not edit by hand)
│   ├── make_toolbar_icons.py # XPM → PNG conversion script (standard library only)
│   ├── dictionaries\      # Hunspell spelling dictionaries (en_US embedded in the executable, others loaded at runtime)
│   ├── formatters\        # Pinned dprint manifest / config / helper entitlement (the -dprint release flavour)
│   └── i18n\              # UI translations (messages.tsv; Traditional Chinese uses the original program text)
├── scripts\
│   ├── fetch_formatter.sh  # Download the pinned dprint CLI and Wasm plugins (versions and SHA-256 from assets/formatters/manifest.json)
│   ├── fetch_formatter.ps1 # Same download for Windows packaging
│   ├── make_appbundle.sh   # Build the macOS MadEdit.app (with icon and dictionaries)
│   ├── make_dmg.sh         # Build the macOS disk image (.dmg, with Applications symlink and signature)
│   ├── install_macos.sh    # Build MadEdit.app in one step and replace the old one in /Applications
│   ├── make_linux_dist.sh  # Build the Linux archive (x86_64 / aarch64, glibc 2.28)
│   ├── make_appimage.sh    # Build the Linux AppImage (x86_64 / aarch64)
│   ├── install_linux.sh    # Build and install to ~/.local in one step
│   ├── make_windows_dist.ps1 # Build the Windows archive (x86_64 / aarch64)
│   ├── changelog_notes.sh  # Extract the release notes of a version from ChangeLog.md
│   └── make_example_js.py  # Generate a synthetic long-line JS sample (example.js) for perf checks
├── src\
│   ├── main.rs             # Entry point: parse the command line, create the window, start eframe
│   ├── app\                # egui UI layer
│   │   ├── mod.rs          # App state, tabs, save / close flow, status bar, main loop
│   │   ├── commands.rs     # Command table (from the original CommandTable, 166 commands, plus Help > Keyboard Shortcuts and Ctrl/⌘+1–9 tab switching) and shortcut parsing
│   │   ├── i18n.rs         # UI language: follow the system or choose manually, looks up messages.tsv
│   │   ├── actions.rs      # Implementation of every command (App::run)
│   │   ├── menus.rs        # Menu bar and toolbar
│   │   ├── icons.rs        # Embedded toolbar icons and their dark-theme variants (black silhouette lightened)
│   │   ├── dialogs.rs      # Options, font, sort, increment numbers, find in files and confirmation dialogs
│   │   ├── form.rs         # Settings page widgets: cards of label / control rows, switches, segmented choices, sidebar and list rows
│   │   ├── view.rs         # Custom-drawn text editor (grid / proportional layout, wrapping, input method, spelling underline)
│   │   ├── hexview.rs      # Custom-drawn hex view
│   │   ├── find.rs         # Find / replace bar and go-to-line bar
│   │   ├── findfiles.rs    # Find / replace in multiple files (background thread)
│   │   ├── macros.rs       # Macro recording and playback (JSON step list)
│   │   ├── print.rs        # Print content model and HTML printing (handed to the browser)
│   │   ├── preview.rs      # In-app print preview window (A4 pages, paging, zoom)
│   │   ├── mdmath.rs       # LaTeX math in the Markdown preview (latex-rust SVG, baseline-aligned inline, centred display)
│   │   ├── mdpreview.rs    # In-app Markdown preview (egui_commonmark + fermaid for ```mermaid diagrams; zoom, fit width, relative-path images; switchable between a separate tab in the main window, side by side or top and bottom, each taking half in split layouts and following the editor scroll)
│   │   ├── fonts.rs        # System font enumeration and the egui font chain
│   │   ├── ui_tests.rs     # egui_kittest keyboard / input method interaction tests
│   │   └── visual_tests.rs # Offscreen rendering screenshots (skipped by default)
│   └── core\               # UI-independent core logic
│       ├── document.rs     # A single document: file I/O, encoding, undo / redo, hex mode
│       ├── dprint.rs       # dprint formatter integration: language map, helper discovery, stdin formatting
│       ├── editor.rs       # Line array, cursor, selection (including rectangular), character grid
│       ├── piece_table.rs  # Piece Table for hex mode and large files (positioned-read page cache)
│       ├── hexbuf.rs       # Hex editing buffer
│       ├── encoding.rs     # Encoding detection, decoding and encoding
│       ├── search.rs       # Search engine (plain text / Regex / Hex)
│       ├── syntax.rs       # syntect syntax highlighting and themes
│       ├── settings.rs     # Settings file (JSON), history, session
│       ├── textops.rs      # Text transforms (case, full/half width, sort, format JSON/XML, etc.)
│       ├── chinese.rs      # Simplified/Traditional Chinese and Japanese Kanji conversion (tables in chinese_tables.rs, from the original)
│       ├── spell.rs        # Spell check
│       └── model.rs        # EditMode, NewLineFormat
├── docs\
│   └── adr\                # Architecture decision records (ADR)
└── AGENTS.md               # Project handover and development guide (this file)
```

---

## 4. Common Development Commands

Run them from the project root (use PowerShell on Windows; bash or zsh on macOS / Linux):

- **Syntax and type check**:
  ```powershell
  make check
  ```
- **Run unit tests** (including egui_kittest interaction tests):
  ```powershell
  make test
  ```
- **Offscreen screenshot** (to check the UI when no window can be opened):
  ```bash
  MADEDIT_SHOT_OUT=out.png MADEDIT_SHOT_FILE=file MADEDIT_SHOT_CMDS=menuHexMode cargo test shot -- --ignored
  ```
- **Package the macOS App and DMG**:
  ```bash
  make dist-macos
  ```
- **Package the Linux archive and AppImage** (needs `cargo-zigbuild` and zig; `ARCH=x86_64` or `aarch64`):
  ```bash
  make dist-appimage
  ```
- **Build and install into /Applications** (removes the old version automatically):
  ```bash
  ./scripts/install_macos.sh
  ```
- **Build and install on the local Linux machine** (Arch / Omarchy; installs to `~/.local`, so the launcher finds it):
  ```bash
  ./scripts/install_linux.sh
  ```
- **Package the Windows archive** (outputs to `dist/`; `-Arch aarch64` selects ARM):
  ```powershell
  powershell -ExecutionPolicy Bypass -File .\scripts\make_windows_dist.ps1
  ```
- **Extract release notes from ChangeLog** (GitHub Release uses the same script):
  ```bash
  make release-notes VERSION=0.5.24
  ```
- **Run the application**:
  ```powershell
  cargo run
  ```
- **Regenerate the toolbar icons** (run after editing `assets/icons-src/*.xpm`):
  ```bash
  python3 assets/make_toolbar_icons.py
  ```

---

## 5. Current Progress

> **Since 2026-10-23 the UI has been rewritten in egui** ([ADR 0011](docs/adr/0011-egui-rewrite.md)). Phases 1–10 below are the history of the Slint era.
> The `.slint` files, `buffer.rs`, `hex.rs` and `memmap2` mentioned there have been removed; their features were reimplemented in the egui version.

- [x] **Syntax cache line identities** (#9): `SyntaxCache::lines_replaced` rebases cached spans and checkpoints after line-count changes and rewinds pending parsing. Document edits, undo and redo report exact removed/inserted counts; decoding a whole document clears parser state.
- [x] **Edit range contracts** (#10): debug-only prefix/suffix fingerprints reject out-of-range edits, including shifted suffixes; empty-document normalization and zero-width insertion ranges remain valid. Release builds do not compute these fingerprints.
- [x] **Whole-edit memory** (#11): undo snapshots transfer changed strings with `truncate`/`drain`. Whole-document or ≥10,000-line edits release reconstructible Unicode original bytes before cloning; no-ops restore them. Malformed, legacy and mixed-newline raw data is retained until a real change is confirmed to preserve lossless reinterpretation.

- [x] **Local test improvements**: deterministic decode-error protection, exact undo/redo and truncation assertions, a 30-case encoding/BOM/newline matrix, seeded Piece Table/document models, and an explicit 1 GiB streaming-save regression. `make test-visual` compares four scenes with bundled test fonts and strict glyph checks; reviewed baselines currently cover macOS only. `make bench` saves raw samples and environment metadata and optionally rejects regressions against a compatible baseline. See [local testing](docs/testing.md). CI/CD remains unchanged.

- [x] **Long single-line performance** (branch `perf/long-line-lag`): files whose lines reach hundreds of KB (minified JSON/JS, logs, `example.js` 1.99 MB / 180 lines / 158,953 B max line) no longer lag on open, typing, caret moves or scrolling.
  - Syntax highlighting is bounded by the **visible window**, not by line length: syntect cannot resume inside a line, so any exact parse costs O(line bytes × grammar rules) (~3 µs/B with `fancy-regex`, ~0.72 µs/B with Oniguruma). A line over `WINDOW_LINE_BYTES` (2 KB) is therefore parsed only around the visible character range — start snapped back onto a `WINDOW_ALIGN` (256 B) grid and onto the nearest `;` `{` `}`, window body `WINDOW_BYTES` (1 KB) — and the rest of the line is drawn in the foreground colour. Both window edges are then aligned to UTF-8 char boundaries: the byte grid can land inside a CJK character, and slicing there panics (covered by `test_long_line_window_respects_char_boundaries`). The skipped prefix is carried as `HighlightSpan::skip_chars` (no text copy). Results are validated by line content hash (`content_hash` in `src/core/mod.rs`), so editing another line, or scrolling inside the same 256 B bucket, does not re-parse it. Such long lines are state-transparent for the per-line parse state, which keeps edits inside them from re-parsing the lines below. See [ADR 0012](docs/adr/0012-long-line-syntax-highlighting.md).
  - `syntect` now uses the Oniguruma backend (`regex-onig`, bundled by `onig_sys`, needs a C compiler such as `cc`; the release cross-builds use the same toolchain as before). Same grammars, 4.2× faster tokenization.
  - Remaining incremental work is unchanged: an edit marks `dirty_from` and re-parsing stops when the parse state matches the pre-edit checkpoint; checkpoints are dense (after every line ≥ 4 KB or every 32 lines); parsing is time-sliced with an 8 ms per-frame budget (`parse_frontier`, foreground placeholders until filled, `highlight_full` for printing, which falls back to a 16 KB prefix on huge lines).
  - Line layout (`chars` + per-character x positions) is cached across frames by content hash (`ViewState::begin_frame` sets the document version and drops entries unused by the previous frame; `clear_frame_layout` after edits), so a 158 KB line is measured once, not once per frame.
  - The wrap cache is incremental: each logical line's break positions are cached with the same `content_hash`, so an edit re-wraps only the changed line; settings/width/font changes still rebuild everything.
  - The draw loop is grouped by logical line: characters, per-character x positions and syntax colors are built once per line per frame instead of once per visual row, which had multiplied the work by hundreds of rows on wrapped long lines.
  - Width sampling in `widths.prepare` is capped (~4 K chars per frame) instead of scanning up to 200 whole lines; spell-check ranges are cached by content hash instead of cloning the whole line as a key; plain-text syntax spans are cached as an empty foreground segment; `SyntaxCache` lines are `Rc`-shared.
  - The status bar's CharPos / SelSize / visual column are memoized per (selection, document version) in `EditorState::status_info`.
  - The benchmark fixture is generated: `python3 scripts/make_example_js.py` writes `example.js` (1.99 MB, 180 lines, longest line 158,953 bytes, with Chinese string literals so byte-aligned windows meet multi-byte characters). It is git-ignored; only the generator is committed. Measured with `MADEDIT_LONGLINE_BENCH_FILE=example.js cargo test --release benchmark_long_line_ -- --ignored --nocapture` (offscreen software rendering, median; not including the native window or GPU presentation). Before / after the windowing + Oniguruma change: keystroke 65.3 / 62.1 → 6.1 / 1.5 ms (NoWrap / ByWindow), End-caret 61.3 / 93.5 → 22.2 / 20.3 ms (the first long jump only; repeating it is a few ms), Shift+End 224.9 / 184.2 → 41.5 / 36.7 ms, 5× PageDown 593.4 / 578.8 → 6.3 / 1.6 ms, click 166.5 / 193.4 → 0.6 / 0.6 ms, open+first render 517.1 / 121.2 → 130.7 / 64.8 ms. Wrapped navigation is covered by `test_wrapped_long_line_rows_caret_and_edit`; the window behaviour by `test_long_line_window_parses_only_visible_region` and `test_long_line_window_follows_scroll`.
- [x] **Proportional fonts and fixed-width mode**: `CharWidths::get` measures a character it has not seen on demand (it keeps the egui context and font from `prepare`) instead of returning the cell width; the line layout and wrap caches are keyed by line content and would otherwise keep the guessed width, which made proportional fonts overlap in newly opened files. `CharWidths::invalidate` (called the frame after `FontManager::apply`, once `set_fonts` is active) bumps a generation in the width key so every cache is rebuilt. That same frame, `fonts::editor_is_proportional` compares the "i" and "M" advances; in fixed-width mode a proportional font opens `Confirm::ProportionalFont`. `fixed_width` now defaults to false. View preferences changed outside Options are saved at once through `App::save_settings` (`saves_view_preference` in `actions.rs` lists the commands; the font, size, tab width and line spacing menus and the Font dialog call it directly).
- [x] **Markdown preview reuses unchanged Mermaid diagrams**: when you edit text, move a diagram or save to another folder, diagrams whose source is identical to the previous version are reused. A diagram is converted again when it changes, when the theme changes or when the layout width crosses a bucket; removed diagrams are still released. The full layout is kept as is; segmented drawing is left for the next phase to evaluate.
  - Set `MADEDIT_MD_BENCH_FILE` and run `cargo test --release benchmark_ -- --ignored --nocapture --test-threads=1` to compare content rebuild time with preview layout time including file open, fonts and diagram rasterization. The latter excludes the native window and GPU presentation; isolate settings with `MADEDIT_CONFIG_DIR` when measuring.
- [x] **Markdown preview scrolling and image improvements**: block start/end and the document end record the source line and display position; the view resynchronizes after edits, zoom, panel width or image loading change the layout, passes in-line progress when long lines wrap, and re-enabling sync aligns with the editor immediately.
  - Sync scrolling works both ways: scrolling the preview or dragging its scrollbar moves the editor back along the existing anchors, preserving the cursor and selection. Both the side-by-side and separate-tab layouts have it on by default and it can be unchecked to read independently. While the preview leads it keeps the exact source progress, so whole-line rounding does not make the two pull on each other, and layout recalculation is not sent back as a manual scroll.
  - The side-by-side splitter keeps its full drag range, so the editor's vertical / horizontal scrollbars cannot steal its events. Switching to side by side gives each half the space and manual resizing is kept afterward. Interaction tests cover real clicks to switch, two-way drag through the middle of the splitter, release, and no snapping back after content updates.
  - The preview no longer writes its own required width back as the panel size; when the main window size update is delayed after the native window closes, each side still keeps half. The last size is also saved when a fast drag's move and release land in the same frame, and tests verify that resizing the main window after manual adjustment does not force it back to half.
  - Ordinary and reference-style images both resolve against the document folder, and reference definitions can span Mermaid blocks. Alt-text formatting and text links that share definitions are preserved. File names with CJK characters, spaces and percent-encoding are supported; load failures show the reason, and "Reload images" rereads them after a missing image is restored or changed externally.
  - Relative-path images in an unsaved document prompt you to save first instead of wrongly using the program's working directory. HTTP images are still not loaded over the network; local image changes are reloaded by the user's button rather than by checking the disk every frame.
- [x] **dprint formatter and split releases** (branch `feat/dprint-formatter-research`):
  - Tools > Convert/Format > "dprint Format" (`src/core/dprint.rs`, command `DprintFormat`) pipes the whole document through a dprint helper found next to the executable (macOS `Contents/Helpers`, portable `formatters/` beside the exe, `target/formatters` in development) or on `PATH`, with `DPRINT_CACHE_DIR` pointed at the user cache. Languages and extensions (17 pinned Wasm plugins, ~56 MB unpacked): clang-format (c cc cpp cxx h hh hpp hxx m mm — required), TypeScript/JavaScript, JSON/JSONC, CSS/SCSS/Sass/Less (malva), HTML/XML/SVG/Vue/Svelte and template languages (markup_fmt), YAML (pretty_yaml), Markdown, TOML, GraphQL (pretty_graphql), SQL, Dockerfile (file name `Dockerfile` too), Shell sh/bash/zsh/mksh/bats incl. rc-file names (shfmt), Python (ruff), Go (gofumpt), PHP (mago), Lua/Luau (stylua) and C#/MSBuild (csharpier; `.xml` goes to markup by order). Unsupported extensions are refused up front; the stdin writer, reader and a 60 s timeout run on threads so large files cannot deadlock.
  - `assets/formatters/manifest.json` is the single source for the pinned dprint 0.64.3 and plugin versions with SHA-256; `scripts/fetch_formatter.sh` / `.ps1` verify and unpack them into `target/formatters`, and `assets/formatters/dprint.json` (4-space indent for C/C++; clang-format 0.2.1 does not accept per-section options yet) ships with the app. `test_bundled_config_lists_exactly_the_supported_plugins` keeps the config and the language table in sync; `dprint_formats_c_via_bundled_helper` (`--ignored`) is the end-to-end smoke after `make formatters`.
  - Every release now ships twice: the clean package (unchanged names) and a `-dprint` package built with `MADEDIT_WITH_FORMATTER=1` (Windows: `-WithFormatter`): `make_linux_dist.sh`/`make_appimage.sh` add `formatters/` next to the binary (install.sh copies it), `make_windows_dist.ps1` adds `formatters\`, and `make_appbundle.sh` adds `Contents/Helpers/dprint` + `Contents/Resources/formatters/`, signs the helper with Hardened Runtime and `assets/formatters/dprint.entitlements` (`allow-unsigned-executable-memory`, needed by wasmtime — the app binary does not get it), and folds the helper's minimum macOS version into `LSMinimumSystemVersion`. `.github/workflows/release.yml` builds both flavours on every platform.
- [x] **Release packaging**: `Makefile`, `ChangeLog.md` and `.github/workflows/release.yml`.
  Linux (tar.gz and AppImage) and Windows are built for x86_64 and aarch64; macOS builds Apple Silicon and Intel DMGs.
  When a `v*` tag is pushed, the Release notes come from the matching ChangeLog section.
- [x] **Full egui rewrite and restoration of original features** (feat/egui-rewrite):
  - Multi-document tabs, undo / redo, system clipboard, settings and session persistence (recent files, cursor positions, window size).
  - User-selectable system fonts and sizes (proportional fonts included), CJK fallback, line spacing, System / Light / Dark theme and syntax theme selection. Dialogs, banners and preview chrome follow the active theme.
 - Modern look (`install_style` in `src/app/mod.rs`): rounded widgets, tabs and windows, soft shadows, card-style find bar and banners, accent primary buttons (`primary_button` in `dialogs.rs`). Vertical spacing stays compact so long menus scroll as little as possible. When the window is wide enough, zoom / theme / quick search sit at the right of the menu bar (`quick_controls` in `menus.rs`), otherwise they fall back to the toolbar. The theme control is one painted icon button (`theme_toggle` / `paint_theme_icon`: sun or crescent for the theme in use) that switches between Light and Dark; its accessible label is "介面主題：<current>", plus " · 跟隨系統" while following the system, since a click ends that. The `ToggleTheme` command does the same (`App::toggle_theme`): it saves the opposite of the theme shown, so the next launch starts in it. `ThemeChoice::System`, the default without a settings file, follows the OS; View > Theme and Options still offer it. The kittest harness sets its own theme after creating the app, so the startup theme is tested by building `App::with_settings` on a plain context. ✕ is in no bundled font, so close buttons use ×; Hack is appended to the proportional fallback for ▾ ▼ ●.
 - Menus go through the `menu` helper in `menus.rs`: top-level menus are at least 240 pt wide, submenus and context menus 180 pt, and every popup scrolls when it would run past the window bottom (egui menus never scroll on their own). Context menus reuse `check_label` so their text lines up with the main menus.
 - The View menu is hierarchical (`view_menu` in `menus.rs`): edit modes, Encoding / Syntax, then Font ▸ / Layout ▸ / Show ▸ submenus, Preview as ▸, and window chrome (language, theme, toolbars, full screen, always on top, post-it). Put new view toggles into the matching submenu rather than the top level.
 - When the CJK fallback lacks Hangul (PingFang TC, Microsoft JhengHei), `fonts.rs` adds a Korean fallback (AppleGothic first, it is much smaller than Apple SD Gothic Neo), checked through the font's character map. Known cost: on such systems about 15 MB more font data stays in memory from startup, because 한국어 is always listed in the language menu and Korean text in the editor needs it too.
 - egui_commonmark draws a block quote's events without its table handling, so a top-level quote that holds a table is stored as `Content::Quote` (`unquote_table_block` strips the `>` markers) and the preview draws the quote bar itself (`quote_frame`). Quotes nested inside it still go through egui_commonmark.
 - Markdown preview tables use a zero minimum column width, because egui's Grid paints stripes at the minimum width while the frame wraps only real content. egui_commonmark forces `item_spacing.x` to 0, so the column gap stays narrow.
 - Toolbar icons (`icons.rs`) are loaded twice: the original art for the light theme, and a dark variant where the black silhouette (outlines and solid black bodies connected to the outside, such as the Find binoculars) is lightened with its hue kept; dark strokes drawn on a light fill inside the icon (text lines, the rim around the lenses) stay dark. Textures use nearest magnification but linear minification, because the 32 px art is shrunk to 16 pt on 1x displays. `test_every_icon_is_visible_on_both_themes` measures outline contrast against both panels and rejects solid patches that blend into the dark panel, so new icons are checked automatically; `MADEDIT_ICON_MONTAGE=out.png cargo test montage -- --ignored` renders every icon on both themes.
 - `MADEDIT_SHOT_CLICK=label1,label2` makes the screenshot test click widgets by exact label (for example `檢視(V)` to capture an open menu).
 - Help > Keyboard Shortcuts (F1, `Dialog::Shortcuts`) lists every bound command from `App::bindings` grouped by menu in two columns balanced by line count (`COMMAND_SECTIONS` and `column_split` in `dialogs.rs`; menus are found by their first command, so keep `commands.rs` ordered by menu, which `test_commands_fall_into_their_menu` checks). The filter matches names, the displayed keys and spelled-out modifiers ("ctrl", "cmd"). A button opens Options > Keys (`OPTIONS_KEYS_TAB`).
 - About shows the app icon, version, platform, settings path (home shown as `~`), the project link and a Copy Info button. The link is `repository` in `Cargo.toml` (`CARGO_PKG_REPOSITORY`), which points at the upstream repository `tenyi/MadEdit`, not a fork. A dialog can hand over to another through `switch_to` in `show_dialogs`.
 - The Options dialog is a settings page (`form.rs`): a sidebar of pages (`form::nav_item`, accessibility role Tab) and titled cards (`form::section`) whose rows put the label (trailing colon dropped by `form::bare`) at the left and the control at the right edge. Booleans are switches (role Switch; clicking anywhere on the row flips them too). The custom widgets draw an accent focus ring for keyboard users. Ids for transient row interactions come from `ui.unique_id()`; sibling `Ui`s share `scope_id()`, so ids built from it clash. small exclusive choices are segmented pills (role RadioButton), and the Keys page is a command list with its keys beside a detail card. Page Setup shares the print cards (`print_options_ui`). Build new settings from these helpers rather than bare checkboxes or grids. Icon-only buttons (×, − / +, A− / A+, toolbar icons) go through `form::icon_button`, which gives them a tooltip and a screen reader name; in lists, put the item in the name ("Close a.txt"). `test_icon_buttons_are_named_for_screen_readers` fails on any button announced as nothing or as symbols only (glyphs with a letter, like "A+", need a check by name); in tests, `dialog(&h)` searches only the open dialog, since toolbar icons share their names with the shortcut list rows. Inside a horizontal row a plain `ui.separator()` stretches to all the height on offer, which makes an auto-sized window grow every frame; give it a size (`add_sized`).
 - Every other dialog and confirmation follows the same style through helpers in `dialogs.rs`: `dialog_width` (fixed width that shrinks with a small main window), `dialog_body` (wraps everything above the footer; scrolls when the main window is too short, so the footer stays visible; its `opened` argument, true on a dialog's first frame as tracked by `App::shown_dialog` / `shown_confirm`, starts it at the top and is also how a dialog focuses its first field once), `form::section` / `form::card` for content, `list_card` for scrolling lists, `hint` for small explanatory text, `confirm_body` for a confirmation's title and text, and `footer` for the right-aligned button row (the first button added is rightmost, so add the `primary_button` first; secondary actions go in a nested left-to-right layout). Destructive choices (Don't Save, Clear, Overwrite, Reload) use `danger_button`. Actions that change `self` run after the window closure, from flags set inside it. `test_every_dialog_keeps_its_buttons_inside_its_window` opens every dialog at 400 × 300 and 1100 × 720; add new dialogs to it.
 - The Options dialog is fixed-size (`OPTIONS_SIZE`, not resizable) so switching tabs does not resize it. `fit_to_window` shrinks it and the Keyboard Shortcuts list (which drops to one column) when the main window is small, keeping the footer visible. While Options is open, `App::run` keeps it in place of any dialog a command would open (the status bar says to close Options first), so its unapplied draft survives; F1 switches it to the Keys tab instead. OK / Apply write back only the fields the dialog changed (`Settings::with_edits`, a three-way merge against the copy the draft started from), so recent files, View toggles, zoom and other settings changed while the dialog is open are not reverted. A pending key capture is cancelled when the page or the selected command changes. Fixed-height scroll areas inside auto-sized windows need `min_scrolled_height` as well as `max_height`; otherwise the window's sizing pass gives the area all the room and clips the footer buttons.
  - macOS input methods (Zhuyin, Cangjie, etc.) with preedit display and candidate window positioning; ⌘ and Option shortcuts follow macOS conventions.
  - 166 commands restored from the original command table: File (save a copy, save all, close left / right / others, read-only flag, recent files list, printing and preview), Edit (cut whole line, join lines, delete empty lines and trailing whitespace, bookmark line operations, hex string conversion, indent, comment, case, full/half width, tabs ⇄ spaces, increment numbers, sort, column alignment, date/time), Search (find / replace in multiple files, jump to matching bracket, previous / next position), View (three word-wrap modes, line numbers, bookmark gutter, ruler, control characters, current line, column guide lines, typewriter mode, RTL mirroring, Markdown / HTML preview), Macro (record, play, play multiple times, save, temporary macro), Tools (options, shortcut reassignment, spell check and personal dictionary, word count, Simplified/Traditional Chinese and Japanese Kanji conversion, JSON / XML formatting, Markdown to HTML, HTML to plain text, AStyle formatting), Window (window list, full screen, always on top; plus Ctrl/⌘+1–8 to switch to the Nth tab and +9 for the last tab, which the original did not have; these are shortcuts only and not listed in menus).
  - Fixed the P0–P3 items in problem.md (input method, dark mode colors, CJK fallback, Hex search panic, encoding corruption, large file editing and search, save failure emptying content, status messages not shown, external change banner, replace not updating bookmarks, check for external modification before save, mmap SIGBUS, temp file names, emoji width, window size not passed into the visible area).
  - Also fixed during the rewrite: current-line background in themes having the same color as comments so text was invisible, programming fonts ligating `->` `=>`, and a panic from the font family not being registered on the first frame.
  - UI language: follow the system, or English, 繁體中文, 简体中文, 日本語, 한국어, Deutsch, Español, Français, Italiano, Português. Translations are in `assets/i18n/messages.tsv`; Traditional Chinese still uses the original text in the program.
- [x] Project skeleton and Git version control initialized.
- [x] Project officially renamed to MadEdit (the `-Rust` package suffix removed):
  - Package name in `Cargo.toml` changed from `MadEdit-Rust` to `MadEdit`; the built executable is named `MadEdit` (`MadEdit.exe` on Windows).
  - The Slint main window title was changed to `MadEdit` as well; the project directory was renamed to `MadEdit`.
  - The project name was synchronized across `AGENTS.md`, `GLOSSARY.md`, the ADRs and the `.scratch/` specs.
- [x] Application icon and taskbar integration:
  - The icons reuse the original MadEdit (madedit-mod): `madedit-icon.png` comes from `madedit-mod.png` (source of the window icon and macOS icns), and `madedit-icon.ico` from `images/Mad2.ico` (Windows executable icon).
  - The Slint `Window` sets the `icon` property so the taskbar / title bar show the dedicated icon.
  - On Windows targets `build.rs` embeds the `.ico` into the `.exe` resources through `embed-resource` (only a warning, not a build failure, when RC.EXE / windres is missing); it is a no-op on other platforms.
  - `assets/madedit.desktop` is provided for Linux desktop environments (launcher / taskbar).
- [x] Slint UI integration (window, toolbar buttons, text editing area, status bar).
- [x] Native cross-platform "Open" and "Save" file dialogs.
- [x] Mode switching UI (Text / Column / Hex buttons and state).
- [x] Column / Block Mode and rectangular selection (Phase 4 done):
  - Introduced `SelectionKind` (Stream and Block) and a two-dimensional visual rectangle model; geometric rectangle slicing and highlighting are based on a fixed character grid with Visual Column (full-width = 2 columns, tab aligned to 4 columns).
  - Supports the toolbar "Column Mode" toggle and, in text mode, Alt+mouse drag / Alt+Shift+arrow keys to trigger rectangular selection.
  - Supports rectangular copy (Ctrl+C), cut (Ctrl+X), column-aligned paste (Ctrl+V), simultaneous multi-line typing and Backspace/Delete block removal, with short-line padding and out-of-bounds safeguards.
  - The status bar shows the rectangle size live (such as `Block: 3x4`).
- [x] Search and replace engine with inline panel (Phase 5 done):
  - Implemented the core search engine (`src/core/search.rs`) supporting plain text, Match Case, Whole Word, regular expressions (Regex) and hex byte (Hex Byte Pattern) search.
  - Implemented circular navigation (`find_next`, `find_prev`) and single / all replace (`replace_current`, `replace_all`), with automatic highlight, scrolling into view and exact replace counts.
  - Implemented a collapsible inline panel at the top of the editing area in Slint, with find box, replace box, checkboxes, live match count (such as `3/15`), previous / next, replace / replace all and close buttons.
  - Full shortcut set: `Ctrl+F` (open the find bar, prefill the selected text and focus), `Ctrl+H` (open find and replace), `Esc` (collapse the panel), `F3` (find next), `Shift+F3` (find previous).
- [x] Syntax highlighting (Phase 6 done):
  - Integrated the `syntect` engine (30+ major languages including Rust, C/C++, Python, JavaScript, TypeScript, JSON, Markdown, HTML, CSS, XML, YAML, SQL) with the `InspiredGitHub` light theme.
  - Automatic language detection by extension, with a "Plain Text" fallback.
  - Visible-line data introduced `HighlightSpan` (`text` and `color`); the Slint custom editor renders multi-colored text segments, fully decoupling the selection geometry layer from the color layer.
  - The toolbar offers a language drop-down and the status bar shows the current syntax name live.
- [x] Go to Line and editing enhancements (Phase 7 done):
  - Implemented a collapsible inline panel for jumping to a line and column (`Ctrl+G` to open, `Enter` to jump, `Esc` to cancel and collapse).
  - Supports `line` and `line:col` syntax with automatic clamping to bounds; in hex mode it switches to Offset jumping (supports `0x` and hexadecimal offset parsing, with zero-copy random page jumps in very large files).
  - Implemented common line editing operations (`Alt+Up` / `Alt+Down` move whole lines, `Ctrl+Shift+D` duplicate line, `Ctrl+Shift+K` delete whole line, `Home` toggling between first non-blank character and line start).
  - `DocumentBuffer` wraps line editing and jumping in one place, keeping the underlying buffer and the UI rendering in sync.
- [x] Hex dump display (Offset column, Hex Byte column, ASCII preview column).
  - Optimized: reads the first 1 MB preview at random directly from the Piece Table, with no whole-file memory copy.
- [x] Line ending conversion and bookmarks (Phase 8 done):
  - Supports switching and two-way conversion between CRLF (Windows), LF (Unix/macOS) and CR (Classic Mac), with atomic save.
  - The toolbar offers a line ending drop-down and the status bar shows the current format name live.
  - Implemented a `BTreeSet`-based line bookmark system with toggle (`Ctrl+F2`), next bookmark (`F2`), previous bookmark (`Shift+F2`) and clear all (`Ctrl+Shift+F2`).
  - Draws a prominent blue dot marker in the left line-number gutter and maintains the bookmark coordinate invariants precisely across line insert, move, copy and delete.
- [x] External file change monitoring and reload (File Watcher & Auto Reload) (Phase 9 done):
  - `DocumentBuffer` tracks the on-disk timestamp (`last_modified`) and implements a dependency-free, standard-library-only `mtime` comparison (`check_external_change`, `reload_file`, `ignore_external_change`).
  - Fixed a Windows issue where small files (<=512 KB) held a memory-mapping handle and caused external write conflicts (Win32 1224): small files are converted to an in-memory PieceTable, which releases the file lock entirely.
  - A lightweight check every 1.5 seconds runs in the Slint main loop and shows a prominent, non-intrusive amber banner at the top of the editing area (`⚠️ The file was modified by another program.`), with an extra dark-red warning if there are unsaved changes; "Reload" and "Ignore" are supported.
  - Reloading preserves the cursor line / column and the bookmark set and applies automatic bounds protection.
- [x] Editor focus input fix and the classic original MadEdit UI (Phase 10 done):
  - Hoisted `editor-focus := FocusScope` to the outer level and added `focus-editor()`, fully fixing the loss of focus that prevented typing after clicking the toolbar or the editing area.
  - Implemented a light/dark dual theme system (follows the OS `Palette.color-scheme` by default, switchable with one click); the `syntect` engine switches between the dark (`base16-ocean.dark`) and light (`InspiredGitHub`) themes in step, and C# (`.cs`), Java, Go, TOML and Shell syntax detection was added.
  - Implemented the four classic original MadEdit interface elements: a top main menu bar plus a compact toolbar (with font size zoom and an inline quick find box), a file tab bar plus a horizontal column ruler (`0..10..20..`), current-line outline highlight plus visible control characters (space `·`, Tab `»`, green end-of-line `↵`), and a 5-cell professional status bar at the bottom (with an `Insert` / `Overwrite` mode toggle).
- [x] Multi-encoding support and auto detection (Phase 1 done):
  - On open, a BOM (UTF-8 / UTF-16LE / UTF-16BE) is recognized first; without a BOM, strict UTF-8 is verified, otherwise it falls back to Big5.
  - The toolbar offers a menu of 10 common encodings plus "Reinterpret" and "Convert" buttons.
  - Fixed silently corrupting data on save when lossless decoding was impossible (`has_decode_error` guard).
  - Fixed UTF-16LE / UTF-16BE being saved as UTF-8 with the BOM lost (own 16-bit encoding and BOM preservation).
- [x] Custom editor component (Phase 2 done):
  - Uses Slint declarative visible-line virtual rendering (`for line in visible-lines`) with `FocusScope` and `TouchArea`, fully replacing the built-in `TextEdit`.
  - Implemented a fixed Character Cell Grid (half-width 1 column, CJK full-width 2 columns, tab expansion aligned to 4 columns), fully solving cursor drift with monospace fonts across platforms.
  - Supports the left line-number gutter, arrow-key navigation, Shift selection extension, mouse click / drag selection, wheel scrolling, text insert and delete, clipboard shortcuts (Ctrl+C/X/V/A) and a live `Ln X, Col Y` display in the status bar.
- [x] Large file and core buffer optimization (Phase 3 done):
  - Introduced `memmap2` read-only memory mapping for opening large files in seconds with zero-copy random page reads.
  - Implemented the Piece Table (Original Buffer + Add Buffer + Piece list) and a lightweight `LineIndex` (newline offset binary search index).
  - Saving streams directly through the piece list, removing whole-file memory use and the cost of re-encoding the whole text on every keystroke.
- [x] Live status bar (length, modified marker, decode warning, mode, line ending and encoding).
- [x] Unit test coverage (hex conversion, line ending detection, Buffer sync, auto encoding detection, UTF-16 round trip, corruption-proof save guard, reinterpret and convert encoding, character grid computation, cursor selection navigation and editing, Piece Table edit / delete, LineIndex on-demand reads and CR/LF/CRLF support, large file mmap instant open, large file edit and atomic save tests, compatibility query interface, character grid visual slice extraction, rectangular selection highlight rendering, column copy / cut / paste / multi-line simultaneous typing, Buffer column mode rendering tests).
- [x] Classic MadEdit style UI redesign (Phase 10 done):
  - Added `ui/theme.slint`, a classic-style component library (colors, `ClassicToolButton`, `ClassicButton`, `ClassicLineEdit`, `ClassicCheckBox`, `ToolbarSeparator`, custom-drawn tooltips).
  - Added `ui/menubar.slint`, the classic menu bar and popup menus (`MenuItemDef` / `MenuDef` data model, separators, check marks, shortcuts, grayed-out disabled items, a single shared `PopupWindow` + `open-index`).
  - Rewrote `ui/appwindow.slint`: classic menu bar → original icon toolbar (6 groups, 21 icons) → tab bar → find / go-to / external change banner → editing area → six-field status bar (Ln / CharPos / SelSize / encoding.DOS / read-only / INS).
  - `src/main.rs` builds 7 top-level menus (File / Edit / Search / View / Tools / Window / Help) from the original `CommandTable`, computes menu widths, x-offsets and popup sizes on the Rust side, and dispatches actions through a single `menu-action(string)` callback.
  - Syntax highlighting / character encoding / line ending / reinterpret / convert encoding moved into menus, opening a generic scrollable selection panel when clicked.
  - Added `assets/icons-src/` (38 original XPM sources + GPL license notes) and `assets/make_toolbar_icons.py` (a standard-library-only XPM→PNG converter).
  - Merged the complete features of another parallel Phase 10 track (see [ADR 0010](docs/adr/0010-classic-ui-consolidation.md)):
    - Dark / light themes (`Theme.is-dark` manages the palette in one place; follows the system color scheme by default and can be toggled from the toolbar).
    - Horizontal column ruler (shifts in sync with the horizontal scrollbar), right vertical scrollbar and bottom horizontal scrollbar (thumb dragging + button nudging).
    - Control character display (space `·`, Tab `»`, end-of-line `↵`) and current-line outline highlight.
    - Insert / overwrite mode (`EditorState::overwrite_mode`, toggled with the `Insert` key, shown as `INS` / `OVR` in the status bar).
    - A resident `FocusScope` handles keyboard events and automatically restores focus after open, save and mode switches (focus fix).
    - Added C#, Java, Go, Shell and TOML syntax detection; `MadEdit <file>` command-line opening.

### Known Issues (handle first)

- macOS port: added a Finder / Dock open-file Apple Event bridge (`src/macos.rs`) that does not replace the winit delegate; filled in platform shortcuts such as ⌘W / ⌘Q / ⌘⇧S and delete-to-line-start, keeping ⌘H / ⌘M / ⌘Tab as system operations.
- macOS packaging and installation finish signature verification before replacing the old App, and support custom Cargo output locations; `scripts/test_macos_bundle.sh` verifies resources, signature and failure protection. `MADEDIT_CONFIG_DIR` isolates settings for native testing, and `--screenshot` does not read or write the user session.
- Quit requests from the native system menu / Dock go through the App's unsaved-changes confirmation; on quit, tabs are kept until the session is finally saved, and cancelling quit no longer closes clean documents early. `python3 scripts/test_macos_runtime.py` verifies text / hex rendering and a normal exit of the real App on a logged-in desktop.
- Touch Bar Mac quit crash: `vendor/accesskit_macos` backports upstream [AccessKit #835](https://github.com/AccessKit/accesskit/pull/835), fixing the accessibility adapter overwriting the AppKit KVO subclass on release. Accessibility support is kept; see `vendor/README.md` for the source, license, tests and removal conditions. Also run `cargo test -p accesskit_macos --test integration` to verify the eight KVO / release regression scenarios.
- Public distribution currently uses an ad-hoc signature, with no Developer ID / Apple notarization yet; local builds run. Intel and other macOS versions still need their own verification; a pass on a local Apple Silicon machine cannot be taken to mean every version passes.

1. ~~**Saving a non-UTF-8 file corrupts it**~~ (fixed in Phase 1: saving is refused when `has_decode_error` is true, and reinterpret and convert encoding are supported).
2. ~~**UTF-16 saves become UTF-8**~~ (fixed in Phase 1: own UTF-16LE/BE byte encoding and BOM preservation implemented).
3. ~~**Large file memory and performance**~~ (fixed in Phase 3: Piece Table + `memmap2` memory mapping, instant open of large files and streaming save, no more whole-file reallocation or whole-text re-encoding).

---

## 6. Roadmap for the Next Agent

The next agent can develop in the following order of priority:

### Phase 1: Multi-encoding switching and conversion (Encoding Support) - [Done]
- Character encoding detection, conversion, corruption-proof save and toolbar integration are done.

### Phase 2: Custom editor component (Custom Editor View) - [Done]
- Slint visible-line virtual rendering, fixed character grid, line-number gutter, cursor / selection and keyboard / mouse interaction are done.

### Phase 3: Large file and core buffer optimization (Large File Engine) - [Done]
- The Piece Table data structure, `memmap2` zero-copy large file open, `LineIndex` and streaming save are done.

### Phase 4: Column mode (Column/Block Mode) and cursor system - [Done]
- The two-dimensional visual rectangle selection model, character grid visual slices, column clipboard (copy / cut / paste), multi-line simultaneous input and delete, and Slint UI Alt-drag integration are done.

### Phase 5: Find and replace (Find & Replace) - [Done]
- The core search engine (plain text / case / whole word / Regex / Hex Byte), circular navigation, replace and replace all are done.
- The collapsible inline Slint panel, live progress (`3/15`), shortcuts (Ctrl+F, Ctrl+H, Esc, F3, Shift+F3) and auto focus are done.

### Phase 6: Syntax highlighting (Syntax Highlighting) - [Done]
- The `syntect` engine (30+ major languages and the InspiredGitHub theme) is integrated.
- Automatic detection on open, the toolbar language switch menu, multi-colored visible-line segment rendering and status bar display are done.

### Phase 7: Go to Line and editing enhancements (Go to Line & Editing Enhancements) - [Done]
- The go-to line / column inline panel and Hex Offset random jumping (Ctrl+G) are done.
- Common code line operations (Alt+Up/Down move lines, Ctrl+Shift+D duplicate line, Ctrl+Shift+K delete line, smart Home) are done.

### Phase 8: Line ending conversion and bookmarks (Line Endings Conversion & Bookmarks) - [Done]
- Two-way CRLF, LF, CR conversion, toolbar menu and save sync are done.
- Editor line bookmarks (Ctrl+F2 toggle, F2 / Shift+F2 circular navigation, Ctrl+Shift+F2 clear all, gutter blue dot marker and coordinate maintenance across line edits) are done.

### Phase 9: External file change monitoring (File Watcher & Auto Reload) - [Done]
- File timestamp (`mtime`) comparison, Windows memory-map unlocking, the Slint timer, the amber banner at the top and cursor / bookmark-preserving reload are done.

### Phase 11: Full egui rewrite and restoration of original features - [Done]
- See section 5 and [ADR 0011](docs/adr/0011-egui-rewrite.md). Not implemented: single instance, file associations, true bidi,
  per-item syntax color customization, multi-line Regex, Python macros (replaced by a JSON step list).

### Phase 10: Classic MadEdit style UI redesign (Classic UI Redesign) - [Done]
- The classic Windows-style menu bar, popup menus, original icon toolbar, tab bar and six-field status bar are done.
- Two-way sync between the menu model and core state (edit mode check marks, disabled / enabled state, width and position computation) is done.
- Reuse of the original MadEdit XPM icons and the standard-library PNG conversion toolchain are done.
- Integration with the other parallel Phase 10 track (dark/light themes, column ruler, vertical / horizontal scrollbars, control characters,
  insert / overwrite mode, focus fix and multi-syntax support) is done (see [ADR 0010](docs/adr/0010-classic-ui-consolidation.md)).

---

## 7. Collaboration Principles for the Next Agent

1. **Follow the KISS principle strictly**: do not introduce unnecessary complexity or abstraction; implement each feature in the most direct and maintainable way.
2. **Language**:
   - Write `AGENTS.md`, `README.md`, commit messages, PR titles / descriptions and new documents in **English**.
   - Write new code comments in **English** and explain intent in detail. Existing Traditional Chinese (`zh-TW`) comments are left as they are; do not translate or rewrite them unless the surrounding code changes anyway.
   - Reply to the maintainer in the language they use.
3. **Keep the tests complete**: add unit tests for new core logic and make sure `cargo test` and `cargo check` pass with zero warnings.
4. **Git conventions**:
   - The main branch is `main`; develop new features on a `feat/<name>` branch.
   - Commit messages are written in **English** and follow Conventional Commits (for example `feat: ...`, `fix: ...`, `refactor: ...`).
5. **Documentation sync**: when a feature is completed or a new limitation is found, update section 5 (progress and known issues) of this file.
6. **README sync**: `README.md` (English) is the source of truth.
   - Whenever `README.md` changes, update every translated README in the same commit (currently `README_zh-TW.md`).
   - When adding a new translation, name it `README_<locale>.md` (for example `README_ja.md`, `README_zh-CN.md`), and add it to the language list at the bottom of every README.
   - Translations must keep the same section structure as `README.md`.
