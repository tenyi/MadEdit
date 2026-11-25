# Moving a window between displays with different scale factors

MadEdit uses a local patch to winit 0.30.13 on Windows. During a title-bar drag, Windows sends the destination position and size in the suggested rectangle of `WM_DPICHANGED`. Recomputing the rectangle against the source monitor can pin the window to the display boundary.

The patch applies the suggested rectangle during an interactive move. The [patch notes](../vendor/winit-pre/PATCHES.md) document the affected handler, scope and upstream issue references. Recheck this behavior when updating winit.

To verify on Windows, drag a restored window between two monitors with different scale factors in both directions. Check that the pointer stays over the title bar, the window reaches the destination monitor and its contents rescale. Repeat with equal scale factors and check resizing and maximize/restore.
