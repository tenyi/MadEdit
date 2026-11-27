# FSEX300.ttf

`FSEX300.ttf` is **Fixedsys Excelsior 3.01**, a freeware font that emulates the classic
Windows "Fixedsys" terminal look, with an extended Unicode/Latin coverage.

- Source file received as `FSEX300（FIXEDSYS模拟字体）.TTF` (v3.01, dated 2015-03-10).
- The font is freeware and freely redistributable; it is embedded into the executable at
  build time via `include_bytes!` in `src/app/fonts.rs` and used as the default editor
  font (16 px) when the user has not chosen a font of their own.
- CJK and other scripts not covered by the font fall back to the platform CJK/Hangul
  fallback chain, unchanged.
