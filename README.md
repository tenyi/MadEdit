# MadEdit (Rust Rewrite)

**A modern, lightweight, fast, cross-platform Text / Column / Hex editor, rebuilt with Rust and egui.**

English | [繁體中文](README_zh-TW.md)

---

## 📖 Background and Vision

The original C++ / wxWidgets **MadEdit** is a classic tool for many developers and data workers (column editing, hex view, and switching between many character encodings). Over time, its heavy C++ cross-platform dependencies and build chain became hard to maintain on modern Windows, macOS and Linux. The author once rebuilt macOS and Linux versions from the MadEdit-Mod source, but they were slow, many features did not work properly outside Windows, and font rendering in particular was inconsistent across platforms after modification. That is why the project was rewritten in Rust.

**MadEdit** is rebuilt with **Rust (Edition 2024)** and the immediate-mode GUI framework **[egui / eframe](https://github.com/emilk/egui)**. It follows the **KISS (Keep It Simple, Stupid)** principle and reproduces the classic MadEdit interface and command set (165 commands, following the original command table) with zero C++ GUI system library dependencies.

> The UI was first built with Slint and rewritten in egui in 2026-10, to let users pick any system font (including proportional fonts) and to handle input methods more completely. See [ADR 0011](docs/adr/0011-egui-rewrite.md).

---

## ✨ Key Features

### 1. Three editing and viewing modes
- **Text Mode**: ordinary text and code editing, with smart-indent Home (`Home`), moving lines up/down, duplicating lines and deleting lines.
- **Column / Block Mode**:
  - Two-dimensional rectangular selection by visual column (1 column for half-width characters, 2 for full-width CJK, tab-aligned).
  - Toggle it from the toolbar, or hold `Alt + mouse drag` / `Alt + Shift + arrow keys` to select a rectangle.
  - Supports **rectangular copy, cut, column-aligned paste, simultaneous multi-line typing and block deletion** (short lines are padded with spaces).
- **Hex Mode**:
  - Classic three-column view: **Offset**, **Hex Bytes** and **ASCII preview**.
  - Supports jumping to a hex offset (such as `0x1A40`) and hex byte pattern search (such as `4D 5A 90`).

### 2. Fonts and input methods
- Pick any system font and size for the editor (proportional fonts included). CJK characters automatically fall back to a system CJK font. With a proportional font, turn off fixed-width mode (View > Font); MadEdit offers to do so when you pick one. A monospaced font draws slightly faster, because a proportional font is drawn one character at a time to keep the caret and selection aligned.
- Supports macOS input methods (Zhuyin, Cangjie, etc.) with preedit display and candidate window positioning.
- The UI language follows the system, or can be set under "View → Interface Language" and in Options to English, 繁體中文, 简体中文, 日本語, 한국어, Deutsch, Español, Français, Italiano or Português. Language names are always written in their own language.

### 3. Multi-encoding support and corruption protection
- Built on `encoding_rs` plus a custom UTF-16 codec. Supports **UTF-8, UTF-8 BOM, UTF-16LE, UTF-16BE, Big5, GB18030, Shift_JIS, EUC-KR, windows-1252, ISO-8859-1** and more.
- **Smart detection on open**: a BOM is recognized first; without a BOM, strict UTF-8 is tried, then it falls back to the configurable fallback encoding (Big5 by default). New documents use UTF-8 by default.
- **Lossless safeguard**: if a file contains bytes that cannot be decoded losslessly, the status bar warns and overwriting is refused. **Reinterpret** and **Convert Encoding** are available.

### 4. Large file engine
- A **Piece Table** reads the original file in cached pages with positioned reads (pread). It does not use mmap, to avoid SIGBUS when a file is truncated externally.
- Files above a size threshold open in Hex Mode, with data staying on disk and read on demand.

### 5. Markdown preview (with Mermaid diagrams)
- Open it from "View → Preview As → Preview as Markdown" or the toolbar preview button. Markdown is rendered in-app (`egui_commonmark`); the button is disabled when the current file is not Markdown.
- ` ```mermaid ` blocks are rendered as diagrams by [**fermaid**](https://github.com/tenyi/fermaid), with no browser or Node.js needed.
  fermaid is a pure-Rust Mermaid rendering engine the author built specifically for the Markdown preview. It supports all current Mermaid diagram types and produces preview images quickly.
- LaTeX math (`$…$` inline, `$$…$$` as a centred block) is rendered by [`latex-rust`](https://crates.io/crates/latex-rust), also pure Rust. Chinese in a top-level `\text{…}` is shown with the preview font; a formula that cannot be rendered is shown as its TeX source.
- Relative image paths resolve against the document's folder. PNG, JPEG, GIF, WebP and SVG are supported.
- Open the preview in a **separate tab in the main window**, or switch to **side by side** or **top and bottom**. Closing the preview tab keeps the source document open. In split layouts the preview and editor each take half, and the preview can **scroll in sync** with the editor.
- Zoom (`Ctrl` / `⌘` + scroll wheel or trackpad pinch) and fit diagrams to width; high-contrast colors (pure black background in dark mode, pure white in light mode).
- Preview state is kept per tab and follows tab switches.
- You can also "Preview as HTML", which hands the page to the system browser.

### 6. The original MadEdit command set
- **Single instance**: launching MadEdit again sends files to the existing window and brings it forward. Files open in tabs; reopening a file selects its existing tab. Launching without files restores the existing window. Instances using different `MADEDIT_CONFIG_DIR` directories remain independent.
- **File**: multi-document tabs, save a copy, save all, recent files, read-only flag, printing and in-app print preview (A4 pages, paging, zoom), session persistence.
- **Edit**: bookmark line operations, hex string conversion, indent, comment, change case, full/half width, tabs to spaces and back, increment numbers, sort, column alignment.
- **Search**: plain text / case / whole word / Regex / Hex find and replace, find and replace in multiple files, jump to matching bracket.
- **View**: word wrap, line numbers, bookmark gutter, ruler, control characters, current line, column guide lines, System / Light / Dark theme, Markdown / HTML preview.
- **Custom syntax and themes**: place Sublime Text `.sublime-syntax` definitions in `syntaxes/` and TextMate `.tmTheme` files in `themes/`, beside `settings.json` (Help > About shows its path; `MADEDIT_CONFIG_DIR` can override the directory). Subfolders are supported; symlinks are skipped. Restart MadEdit after installing files.
  - Custom definitions take precedence over bundled extension matches and approximate language fallbacks. Choose them from View > Syntax; themes appear in the Syntax Highlighting dialog using the file name without `.tmTheme`. A matching name overrides a bundled resource.
  - Malformed files are skipped individually with a diagnostic on stderr; valid resources and built-ins remain available.
- **Macros and tools**: macro record and playback, shortcut reassignment and a searchable shortcut list (Help > Keyboard Shortcuts, F1), spell check, word count, Simplified/Traditional Chinese and Japanese Kanji conversion, JSON / XML formatting, and a dprint-based formatter (Tools > Convert/Format > "dprint Format") for C/C++/Objective-C (clang-format), JavaScript/TypeScript, JSON/JSONC, CSS/SCSS, HTML/XML/SVG/Vue/Svelte, YAML, Markdown, TOML, GraphQL, SQL, Dockerfile, Shell, Python, Go, PHP, Lua and C#.

### 7. macOS native integration
- Supports Finder "Open With" and dropping files on the Dock icon (including names with CJK characters, spaces and percent-encoding). Opening a file while the app is running adds a tab in the same app.
- Quit from the system menu or the Dock also asks about unsaved changes first.
- Shortcuts follow macOS conventions (⌘W, ⌘Q, ⌘⇧S, ⌘⌥F, Control+Tab, etc.); ⌘H, ⌘M and ⌘Tab are left to the system.

---

## 🏗️ Project Structure

```text
MadEdit/
├── Cargo.toml / Cargo.lock
├── Makefile                # Development, packaging and release-notes commands
├── ChangeLog.md            # Change log; GitHub Release notes come from the matching version section
├── build.rs                # Embeds the icon into the Windows executable
├── .github/workflows/release.yml  # Tag release: Linux, AppImage, macOS DMG, Windows
├── assets/                 # App icons, toolbar icons (from the original), spelling dictionaries
├── scripts/
│   ├── make_appbundle.sh   # Build the macOS MadEdit.app
│   ├── make_dmg.sh         # Build the macOS DMG
│   ├── install_macos.sh    # Build and install MadEdit.app into /Applications
│   ├── test_macos_bundle.sh / test_macos_runtime.py  # macOS packaging and native runtime regression tests
│   ├── make_linux_dist.sh  # Build the Linux archive (x86_64 / aarch64)
│   ├── make_appimage.sh    # Build the Linux AppImage (x86_64 / aarch64)
│   ├── install_linux.sh    # Build and install to ~/.local
│   ├── make_windows_dist.ps1 # Build the Windows archive (x86_64 / aarch64)
│   └── changelog_notes.sh  # Extract the release notes of a version from ChangeLog.md
├── src/
│   ├── main.rs             # Entry point: parse the command line, create the window, start eframe
│   ├── macos.rs            # macOS Finder / Dock open-file and quit event bridge
│   ├── app/                # egui UI layer (menus, toolbar, custom editor, Hex view, Markdown preview, dialogs, command table)
│   └── core/               # UI-independent core logic (document, encoding, Piece Table, search, syntax, settings)
├── vendor/accesskit_macos/ # AccessKit with the Touch Bar quit-crash fix backported (see vendor/README.md)
├── docs/                   # Architecture decision records (ADR), domain docs and macOS validation notes
├── GLOSSARY.md             # Project domain glossary
└── AGENTS.md               # Guide for AI collaboration and development
```

---

## 🚀 Build and Run

### Requirements
- **Rust toolchain**: Rust `1.99` or newer (the minimum for latex-rust, which renders math in the Markdown preview)
- Supported platforms: **Windows**, **macOS**, **Linux**

### Development and test commands

```bash
make check             # cargo check
make test              # unit tests and egui_kittest UI interaction tests
make run               # run the development build
make build             # optimized release build (cargo build --release --locked)
```

`make help` lists all targets. `cargo check`, `cargo test` and `cargo run` still work directly.

See [local testing](docs/testing.md) for focused core/UI tests, reviewed offscreen
baselines (`make test-visual`), the explicit 1 GiB regression (`make test-large`),
and reproducible performance reports with baseline comparison (`make bench`).

After building, the standalone executable is at `target/release/MadEdit` (`target/release/MadEdit.exe` on Windows).

### Building on macOS

A GUI program on macOS must live inside an `.app` bundle to show the Dock icon and menu bar name correctly and to interact with the system.

1. Install the Xcode Command Line Tools (they provide `sips`, `iconutil` and `codesign`):
   ```bash
   xcode-select --install
   ```
2. Build the app and the DMG:
   ```bash
   make dist-macos
   ```
   This is the same as running `./scripts/make_appbundle.sh` and then `./scripts/make_dmg.sh`.
3. The outputs are `dist/MadEdit.app` (with icon and spelling dictionaries, ad-hoc signed) and `dist/MadEdit-<version>-macOS-<arch>.dmg`. Open the app directly or drag it into the Applications folder:
   ```bash
   open dist/MadEdit.app
   ```

The scripts build for the local architecture (Apple Silicon or Intel) and support a custom Cargo target-dir / build.target.
If you already have a universal binary, package it with `MADEDIT_BIN=/path/to/MadEdit ./scripts/make_appbundle.sh`.
The app contains the icon and dictionaries; the old version is replaced only after packaging, signing and verification all succeed.

To install into `/Applications`, run `./scripts/install_macos.sh`; use
`DEST="$HOME/Applications/MadEdit.app" ./scripts/install_macos.sh` to choose another location.

Finder "Open With", dropping files on the Dock icon, and
`open -a "$PWD/dist/MadEdit.app" file.txt` are supported. If the app is already running, the file opens in a new tab.
Common shortcuts: ⌘W closes a tab, ⌘Q quits (asking first if there are unsaved changes), ⌘⇧S Save As, ⌘⌥F Replace,
⌘/ comment, Control+Tab switches tabs; Option+arrows move by word, ⌘+arrows jump to line start/end or document start/end.
⌘H, ⌘M and ⌘Tab are left to the system.

macOS packaging regression test: `./scripts/test_macos_bundle.sh` (does not overwrite an installed app).
On a logged-in macOS desktop, `python3 scripts/test_macos_runtime.py` launches the real app repeatedly,
verifies text / binary document rendering and a clean exit, and keeps screenshots and logs in `dist/validation/runtime/`.
See [macOS port validation](docs/macos-validation.md) for the platforms, results and scope of validation.
Settings are stored by default in `~/Library/Application Support/MadEdit/settings.json`; set the environment variable
`MADEDIT_CONFIG_DIR` to use a separate settings directory. `--screenshot out.png` uses default settings and exits automatically,
without reading or writing your everyday session, which is useful for native window verification.

> Because the app is not notarized by Apple, a downloaded copy may be blocked by Gatekeeper the first time you open it.
> Right-click it in Finder and choose "Open", or run `xattr -cr MadEdit.app` to remove the quarantine attribute.

### Downloads

Release builds are on the [Releases](https://github.com/tenyi/MadEdit/releases) page. `<version>` in the file names matches `version` in `Cargo.toml`.

| Platform | File name |
| :--- | :--- |
| Linux x86_64 | `MadEdit-<version>-linux-x86_64.tar.gz` |
| Linux ARM | `MadEdit-<version>-linux-aarch64.tar.gz` |
| Linux AppImage x86_64 | `MadEdit-<version>-linux-x86_64.AppImage` |
| Linux AppImage ARM | `MadEdit-<version>-linux-aarch64.AppImage` |
| macOS Apple Silicon | `MadEdit-<version>-macOS-arm64.dmg` |
| macOS Intel | `MadEdit-<version>-macOS-x86_64.dmg` |
| Windows x64 | `MadEdit-<version>-windows-x86_64.zip` |
| Windows ARM | `MadEdit-<version>-windows-aarch64.zip` |

Each release file has a matching `.sha256` next to it. After extracting a Linux archive, run `./MadEdit`, or `./install.sh` to install into `~/.local`. Make an AppImage executable and run it; it needs a Vulkan or OpenGL driver on the system. On Windows, extract and run `MadEdit.exe`. On macOS, open the DMG and drag `MadEdit.app` into Applications.

**Two flavours per platform.** The files above are the clean version without any formatter. Every one of them also exists with a `-dprint` suffix (for example `MadEdit-<version>-macOS-arm64-dprint.dmg`, `MadEdit-<version>-linux-x86_64-dprint.AppImage`, `MadEdit-<version>-windows-x86_64-dprint.zip`); that package additionally bundles a pinned dprint CLI and Wasm plugins for C/C++ (clang-format), JavaScript/TypeScript, JSON/JSONC, CSS/SCSS, HTML/XML/SVG/Vue/Svelte, YAML, Markdown, TOML, GraphQL, SQL, Dockerfile, Shell, Python, Go, PHP, Lua and C#, so Tools > "dprint Format" works offline. The clean version still offers the command when a `dprint` executable is found on `PATH`; unsupported extensions are refused with a message. To package locally, add `MADEDIT_WITH_FORMATTER=1` to any `make dist-*` target (or `-WithFormatter` on the Windows script); `make formatters` downloads the pinned dprint into `target/formatters` and is run automatically when needed.

### Packaging for Windows

On Windows (x64 or ARM, named after the local architecture):

```powershell
make dist-windows
```

Or:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\make_windows_dist.ps1
powershell -ExecutionPolicy Bypass -File .\scripts\make_windows_dist.ps1 -Arch aarch64
```

### Packaging for Linux

Use [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) to pin glibc 2.28, so the same executable runs on older distributions (such as Ubuntu 20.04):

```bash
cargo install cargo-zigbuild
# Install zig 0.16 (via the system package manager, or uv tool install ziglang)
make dist-linux       # tar.gz, local architecture
make dist-appimage    # AppImage, plus a tar.gz
make dist-linux ARCH=aarch64
make dist-appimage ARCH=x86_64
```

`./scripts/make_linux_dist.sh [output-dir] [arch]` and `./scripts/make_appimage.sh` are the underlying scripts of those two targets. The architecture is `x86_64` or `aarch64`.

### Releasing

Changes are recorded in [ChangeLog.md](ChangeLog.md). After a version tag is pushed, [`.github/workflows/release.yml`](.github/workflows/release.yml) builds every file in the table above and uses that version's section verbatim as the GitHub Release notes. A tag containing a hyphen (such as `v0.6.0-rc.1`) is marked as a pre-release.

1. In `ChangeLog.md`, move the entries under `## [Unreleased]` into a new section titled `## [x.y.z] - YYYY-MM-DD`. The section must not be empty.
2. Set `version` in `Cargo.toml` to the same `x.y.z` and update `Cargo.lock` (`cargo check` rewrites the lock file).
3. Check that the notes can be extracted on their own (replace `0.6.0` with this release's version):
   ```bash
   make release-notes VERSION=0.6.0
   ```
4. Commit, tag and push:
   ```bash
   git tag v0.6.0
   git push origin main
   git push origin v0.6.0
   ```
   The tag version must equal `Cargo.toml` (`v0.6.0` for `0.6.0`). If it does not, or ChangeLog has no such section, the workflow stops before packaging.
5. When Actions finishes, the Release page shows Linux (x64 / ARM tar.gz and AppImage), macOS DMG (Apple Silicon and Intel) and Windows zip (x64 / ARM) — each in the clean flavour and the `-dprint` flavour — with the ChangeLog section as the notes.

To try packaging without creating a Release, run the Release workflow manually from the GitHub Actions page. The outputs stay in that run's artifacts and are not published.

---

## 🛠️ Tech Stack

| Area | Crate / Technology | Purpose |
| :--- | :--- | :--- |
| **GUI framework** | [`eframe` / `egui`](https://github.com/emilk/egui) (`v0.37`) | Immediate-mode GUI, custom editor and Hex view, input method events |
| **System fonts** | [`fontique`](https://crates.io/crates/fontique) | Enumerate and load system fonts |
| **Markdown preview** | [`egui_commonmark`](https://crates.io/crates/egui_commonmark) | Render Markdown in-app |
| **Mermaid diagrams** | [`fermaid`](https://github.com/tenyi/fermaid) + [`resvg`](https://crates.io/crates/resvg) | In-house pure-Rust Mermaid engine supporting all Mermaid diagram types, converted to SVG and rasterized |
| **Math formulas** | [`latex-rust`](https://crates.io/crates/latex-rust) | Pure-Rust LaTeX math layout to SVG (bundled STIX Two Math font), rasterized like the diagrams |
| **macOS integration** | [`objc2`](https://crates.io/crates/objc2) | Finder / Dock open-file and quit events |
| **Character encoding** | [`encoding_rs`](https://crates.io/crates/encoding_rs) | Multi-encoding conversion and lossless verification |
| **Syntax highlighting** | [`syntect`](https://crates.io/crates/syntect) | Sublime Text syntax definitions and themes |
| **Regular expressions** | [`regex`](https://crates.io/crates/regex) | Regex search and group replacement |
| **Spell check** | [`spellbook`](https://crates.io/crates/spellbook) | Hunspell dictionary spell checking |
| **System dialogs** | [`rfd`](https://crates.io/crates/rfd) | Native open and save file dialogs |
| **Clipboard** | [`arboard`](https://crates.io/crates/arboard) | Cross-platform system clipboard |

---

## 🌐 Documentation Languages

- [English](README.md) (primary)
- [繁體中文](README_zh-TW.md)

`README.md` is the source of truth. When it changes, update every translated README in the same commit.

---

## 🙏 Acknowledgements

Thanks to Alston Chen, the author of the original **MadEdit**, to Minggang Li, the maintainer of **MadEdit-Mod**, and to all contributors. The icons, toolbar icons, Simplified/Traditional Chinese tables and spelling dictionaries in this project all come from the original. This rewrite would not exist without their years of work.

---

## 📄 License

This project uses a free and open-source license in keeping with the spirit of the original. Issues and Pull Requests are welcome!
