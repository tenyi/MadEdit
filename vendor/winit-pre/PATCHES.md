# MadEdit patches for winit 0.30.13

Cross-monitor dragging on mixed-DPI multi-monitor setups: applying the OS-suggested
rect from `WM_DPICHANGED` verbatim while the window is being dragged, instead of
recomputing the rect and nudging it toward the "new DPI" monitor.

## Symptom

On two monitors with different scale factors, dragging the title bar across the
monitor boundary changes the window size but the window never completes the move:
it stays glued to the source monitor's edge (or straddles the boundary). Native
Windows apps and apps using other windowing stacks (for example GPUI's Win32
backend) do not have this problem.

## Cause

winit 0.30.13's `WM_DPICHANGED` handler discards the OS-suggested rect from
`lparam`, recomputes the size from the old logical size, and then runs a
"nudge toward the new-DPI monitor" block whose monitor comparison is inverted
(`MonitorFromWindow` still returns the source monitor at that point). The result
is a rect that straddles the boundary instead of completing the move.

References:

- Diagnosis: [winit #4573](https://github.com/rust-windowing/winit/issues/4573)
- Related resize feedback loop: [winit #4041](https://github.com/rust-windowing/winit/issues/4041),
  still present in 0.30.13 per [winit #4600](https://github.com/rust-windowing/winit/issues/4600)

## The patch

One addition in `src/platform_impl/windows/event_loop.rs`, inside the
`WM_DPICHANGED` arm after the `ScaleFactorChanged` event has been delivered:
when the window is being dragged (`MARKER_IN_SIZE_MOVE` set) and resizing is
allowed (not maximized/fullscreen), call `SetWindowPos` with the suggested rect
and return, skipping the recomputed rect. Everything else is upstream code.
`ScaleFactorChanged` is delivered before the early return, so egui-winit still
updates `pixels_per_point` (it only reads the scale factor; it never writes to
the `inner_size_writer`). This mirrors GPUI's Win32 backend
(`vendor/gpui-pre-windows/src/events.rs` in the Argus project), which applies
the suggested rect directly and drags correctly on the same setups.

Non-drag DPI changes (for example changing display scaling while the window
sits still) keep the upstream path unchanged.

## Provenance

- Base: crates.io `winit 0.30.13` (the version eframe 0.37 requires; 0.30.13 is
  the last stable 0.30 release, and the fix has only landed on `winit` master
  for 0.31).
- Upstream license (Apache-2.0) and package metadata retained; only the one
  addition described above differs from the published crate.
- The package-local lockfile and `.cargo*` metadata are omitted; dependency
  resolution uses the workspace lockfile (`Cargo.toml`
  `[patch.crates-io] winit = { path = "vendor/winit-pre" }`).

## Verification

Manual, on a mixed-DPI dual-monitor setup (this scenario cannot run in a
headless test harness): drag the title bar across the boundary in both
directions, drag back, resize across the boundary, and change display scaling
while the window sits on one monitor. The window should complete the move,
keep a sensible size, and the UI should render at the destination monitor's
scale factor.

## Removal

Remove this vendored copy and the `[patch.crates-io]` entry once MadEdit
upgrades to an eframe release that depends on winit with the fix (0.31 line or
the next stable 0.30.x if one ships it).
