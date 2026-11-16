# 混合 DPI 双显示器拖动问题的定位与修复

> 适用场景：基于 **eframe/egui + winit 0.30.x**（Windows）的 Rust GUI 程序，在两台缩放比例不同的显示器之间拖动窗口时，窗口大小会变，但拖不进另一台显示器（位置被"弹回"或卡在屏幕边界上）。
>
> 修复样本：MadEdit（`vendor/winit-pre` + `[patch.crates-io]`），参照 Argus 项目（GPUI Win32 后端）的正确处理方式。

## 1. 问题现象

- 按住标题栏把窗口从显示器 A 拖到显示器 B；
- 跨过屏幕边界瞬间窗口**大小变了**（触发了一次 DPI 重算）；
- 但窗口**位置回不来/过不去**：要么贴在 A 的边缘，要么横跨边界一半对一半；
- 只有缩放比例不同的显示器之间出问题；缩放一致的显示器之间拖动正常。

## 2. 根因

窗口跨显示器移动时，Windows 会发 `WM_DPICHANGED`：

- `wparam`：新的 DPI；
- `lparam`：**系统建议的矩形**（已经在目标显示器的坐标系里算好了位置和尺寸，保证光标相对标题栏的位置大致不变）。

**正确的处理**就是把这个建议矩形原样 `SetWindowPos`。Windows 自家程序和 GPUI 都是这么做的（见 §3）。

**winit 0.30.13 的处理错在两处**（`src/platform_impl/windows/event_loop.rs` 的 `WM_DPICHANGED` 分支）：

1. 丢弃建议矩形，用"旧物理尺寸 ÷ 旧缩放 × 新缩放"自己重算尺寸 —— 忽略了系统建议的位置；
2. 随后有一个"如果矩形不在新 DPI 显示器上就 1px 步进推过去"的 nudge 块，但里面的
   `MonitorFromWindow` 在此时刻返回的仍是**源显示器**（窗口还没被移动），比较结果永远
   "不匹配"，于是把矩形往源显示器方向推，最终停在一个横跨边界的位置。

症状与上游诊断完全吻合：

- winit #4573（诊断 + 消息跟踪）：https://github.com/rust-windowing/winit/issues/4573
- winit #4041（24H2 混合 DPI resize 回环）：https://github.com/rust-windowing/winit/issues/4041
- winit #4600（0.30.13 仍有问题）：https://github.com/rust-windowing/winit/issues/4600

关键版本事实：**0.30.13 是 winit 0.30 系最后一个稳定版**，修复只在 master（0.31 beta）；eframe 0.37 精确依赖 winit 0.30.13，所以"等 eframe 升级"之前只能自己补。

## 3. 正确的处理方式（参照实现）

Argus 的 GPUI Win32 后端（`vendor/gpui-pre-windows/src/events.rs`）：

```rust
WM_DPICHANGED => self.handle_dpi_changed_msg(handle, wparam, lparam),

fn handle_dpi_changed_msg(...) {
    // ...更新 scale 状态...
    if is_maximized {
        // 最大化窗口：贴目标显示器工作区
        // （MonitorFromWindow + GetMonitorInfoW + SetWindowPos 到 work_area）
    } else {
        // 非最大化：直接采用系统建议的矩形
        let rect = unsafe { &*(lparam.0 as *const RECT) };
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        unsafe {
            SetWindowPos(handle, None, rect.left, rect.top, width, height,
                         SWP_NOZORDER | SWP_NOACTIVATE)
        }
    }
    Some(0)
}
```

要点：**不自己算尺寸、不做显示器间 nudge，信系统的建议矩形**。

## 4. 修复方案：vendor 补丁（不改 eframe，不升 beta）

winit 被 eframe 锁死在 `^0.30.13`，无法直接换 0.31 beta。做法是把 registry 里的
winit 0.30.13 源码拷进仓库，打上最小补丁，用 `[patch.crates-io]` 替换。

### 步骤

1. **拷贝源码**

   ```powershell
   $src = "$env:USERPROFILE\.cargo\registry\src\<registry-hash>\winit-0.30.13"
   Copy-Item "$src\*" vendor\winit-pre -Recurse -Force
   Remove-Item vendor\winit-pre\Cargo.lock, vendor\winit-pre\.cargo-ok, `
               vendor\winit-pre\.cargo_vcs_info.json -Force
   ```

   （`.cargo-ok` 等是 registry 的解压标记，vendor 里不需要；保留上游 `LICENSE`。）

2. **打补丁**：在 `vendor/winit-pre/src/platform_impl/windows/event_loop.rs` 的
   `WM_DPICHANGED` 分支里，紧接上游 `let new_physical_inner_size = ...; drop(new_inner_size);`
   之后插入：

   ```rust
   // MADEDIT PATCH (rust-windowing/winit#4573): while the user drags the window
   // across monitors, apply the OS-suggested rect verbatim. The recomputed rect
   // below gets nudged back toward the source monitor on mixed-DPI setups, so
   // the window straddles the boundary and the move never completes. Apply the
   // suggested rect and return; `ScaleFactorChanged` was already delivered above,
   // so the UI layer still learns the new pixels-per-point. Remove once winit
   // ships the fix (0.31).
   if allow_resize && {
       let window_state = userdata.window_state_lock();
       window_state.window_flags().contains(WindowFlags::MARKER_IN_SIZE_MOVE)
   } {
       unsafe {
           SetWindowPos(
               window,
               0,
               suggested_rect.left,
               suggested_rect.top,
               suggested_rect.right - suggested_rect.left,
               suggested_rect.bottom - suggested_rect.top,
               SWP_NOZORDER | SWP_NOACTIVATE,
           )
       };
       result = ProcResult::Value(0);
       return;
   }
   ```

   设计取舍：

   - **只在拖动中生效**（`MARKER_IN_SIZE_MOVE` 在 `WM_ENTERSIZEMOVE`..`WM_EXITSIZEMOVE`
     之间置位）：静止时的 DPI 变化（改显示设置）保持上游行为，补丁面最小；
   - **`allow_resize` 才生效**：最大化/全屏窗口走上游逻辑；
   - 插入点在上游发送 `ScaleFactorChanged` **之后**：UI 层（egui-winit）拿得到新的
     `pixels_per_point`，渲染缩放不受影响。已核实 egui-winit 0.37 对该事件只读
     `scale_factor`，从不写回 `inner_size_writer`，所以丢弃写回句柄是安全的；
   - `return` 与上游同分支里"新旧缩放相等"时的提前返回写法一致。

3. **接线**

   ```toml
   # Cargo.toml
   [patch.crates-io]
   winit = { path = "vendor/winit-pre" }
   ```

   版本号保持 `0.30.13` 不动，满足 eframe 的 `^0.30.13` 要求。验证补丁生效：
   构建日志应出现 `Compiling winit v0.30.13 (<仓库>\vendor\winit-pre)`，
   且 `Cargo.lock` 中 winit 条目不再有 `source = "registry ..."` 行。

4. **写来源与移除条件**（`vendor/winit-pre/PATCHES.md`）：症状、根因、补丁内容、
   基线来源（crates.io 版本 + 上游 license 保留）、验证方法、移除条件
   （eframe 升级到带修复的 winit 后删除整个目录与 patch 条目）。

## 5. 验证清单（无头环境测不了，必须手工）

- [ ] 两个方向的跨屏拖动都能完成（A→B、B→A）；
- [ ] 拖回原显示器正常；
- [ ] 跨屏拖动过程中窗口尺寸合理（≈原逻辑尺寸），UI 按目标显示器的缩放清晰渲染；
- [ ] 静止状态下改显示缩放比例，窗口行为与打补丁前一致；
- [ ] 最大化窗口跨屏（Win+Shift+方向键）行为不劣化；
- [ ] 回归：全量测试套件无新增失败。

## 6. 迁移到其他项目

| 你的项目用的窗口栈 | 处理方式 |
| --- | --- |
| eframe/egui + winit 0.30.x | 照本文 §4 做 vendor 补丁（改动只有一处，可整段照抄） |
| 自己写 Win32 消息循环 / GPUI / tao 等自管后端 | 检查自己的 `WM_DPICHANGED`：非最大化时直接用 `lparam` 建议矩形 `SetWindowPos`，最大化时贴目标显示器工作区；不要自己算尺寸、不要 nudge |
| tauri/wry、slint、flutter 等自带上层 | 问题可能同样存在于其依赖的 winit 版本，先确认版本再决定是升级还是 vendor 补丁 |

通用原则一句话：**`WM_DPICHANGED` 的 `lparam` 是 Windows 按目标显示器算好的答案，照抄它；任何"自己重算+修正"都可能在混合 DPI 下把窗口推回源显示器。**

## 7. 移除

上游 winit 0.31（或含修复的 0.30.x 后续版）进入你依赖的 eframe 版本后：删除
`vendor/winit-pre/` 与 `[patch.crates-io]` 条目，重新跑一次 §5 清单确认即可。
