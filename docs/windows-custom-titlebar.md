# Windows title bar

The Windows viewport disables native decorations. The title bar puts the application icon and menus on the left and quick controls and caption buttons on the right. Other platforms keep their native decorations.

## Layout and input

The top panel reserves its height with `exact_size`. Derive that height from the menu style so that repeated layout passes keep the same size. Allocate the right-hand controls first and explicitly select left-to-right layout for the menus. Keep the menus inside an egui menu bar to preserve hover switching.

Register the drag area with `interact` before registering interactive controls. Allocating a separate full-width rectangle would advance the layout cursor past the menus. egui handles title-bar dragging, double-click maximize/restore, caption-button clicks and hover. Give each caption button a translated accessible name.

The [Win32 subclass](../src/app/titlebar.rs) handles edge and corner hit testing for resizing the undecorated window. Native caption-button hit tests compete with egui for hover and clicks. Keep the two sets of controls from claiming the same region.

## Verification

Check menu switching, drag and double-click on empty title-bar space, caption buttons, all four resize edges and corners, and quick-control visibility in a narrow window. Verify native snapping and [cross-monitor dragging](mixed-dpi-drag-fix.md) on Windows. The [interaction tests](../src/app/ui_tests.rs) cover egui layout and controls.
