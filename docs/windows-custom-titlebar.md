# Windows 自绘标题栏（把菜单列移进标题栏）的实现笔记

> 适用场景：**eframe/egui 0.37 + winit 0.30.x（Windows）**，想做出 VS Code 那种"图标 + 菜单列 + 右侧快捷控件 + 窗口按钮"的自绘标题栏。
>
> 样本代码：MadEdit 的 `src/app/mod.rs::title_bar`、`src/app/titlebar.rs`、`src/main.rs`。本文记录**最终做法**、**每个坑的根因**和**可复用的判定结论**，方便以后在别处（其它项目或其它窗口）照抄。

## 1. 最终形态与职责划分

一行搞定所有东西：

```
[图标] [菜单1][菜单2]……[菜单8]      …剩余空白…      [快捷控件][分隔线][─][□][×]
└──────────────────── 标题栏（高度 = 菜单自然高 × 1.5） ───────────────────┘
```

| 层 | 负责 | 文件 |
| --- | --- | --- |
| winit 视窗选项 | 去掉原生外框（`with_decorations(false)`） | `src/main.rs` |
| egui 面板 | 标题栏高度、图标、菜单、快捷控件、窗口按钮、拖动区 | `src/app/mod.rs::title_bar` |
| 菜单本体 | 八个下拉菜单（不含容器） | `src/app/menus.rs::menus` |
| Win32 子类 | **只**补回客户区边缘的缩放命中（`WM_NCHITTEST`） | `src/app/titlebar.rs` |

关键分工原则：**拖动、双击最大化、窗口按钮、悬停效果全部交给 egui**，原生层只做 egui 做不到的事（8px 边缘缩放，因为 `with_decorations(false)` 之后 winit 整个客户区都回 `HTCLIENT`）。早期版本让原生层接管视窗按钮（`HTMINBUTTON` 等），结果原生命中与 egui hover 互相打架，hover 闪烁、点击丢失，最后全部撤掉。

## 2. 实现要点

### 2.1 去掉原生外框

```rust
// src/main.rs
#[cfg(windows)]
{
    viewport = viewport.with_decorations(false);
}
```

注意：这样做的代价是原生标题栏、任务栏标题、Snap Layouts 提示全部要自己处理或放弃（见 §3.10、§3.6）。

### 2.2 面板高度：`exact_size`，不要 `set_height`

```rust
let text_h = ui.text_style_height(&egui::TextStyle::Button);      // 15.0（默认字号）
let menu_h = (text_h + 2.0 * ui.spacing().button_padding.y)       // 菜单按钮自然高
    .max(ui.spacing().interact_size.y);                           // 20.0
let bar_height = (menu_h * 1.5).max(24.0);                        // 30.0

egui::Panel::top("titlebar")
    .exact_size(bar_height)          // ← 必须：给定外高，面板保留高度才可靠
    .show_separator_line(false)
    .show(ui, |ui| { /* 内容 */ });
```

- 高度**只能由字体/样式推导**，绝不能测量"上一帧布局结果"（§3.4 的逐帧放大就是这么来的）。
- 面板默认 frame 是 `Frame::side_top_panel`（内距 `(8, 2)`），所以 `bar_height = 30` 时内容区实际是 `2..28`；20 高的按钮行居中后位于 `5..25`，上下各余 3px，视觉刚好。
- 改字号（Options → 界面字体）时 `text_h` 变化，栏高自动跟着变，无需魔法数字。

### 2.3 内容布局：外层 RTL，内层 LTR

```rust
// 外层从右往左：窗口按钮、快捷控件贴右缘
ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        // 关闭 → 最大化 → 最小化（从右往左依次添加，视觉顺序才是 ─ □ ×）
        // 分隔线 + quick_controls(ui, true)

        // 内层从左往右：图标 + 菜单
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            // 图标 allocate_exact_size + painter.image
            egui::MenuBar::new().ui(ui, |ui| self.menus(ui));
        });
    });
```

两个坑都在这里：`scope` 会**继承**父布局方向（§3.8），所以左右两块必须各自显式 `with_layout`；菜单必须包在 `egui::MenuBar` 里（§3.5）。

### 2.4 拖动区：`interact`，不是 `allocate_rect`

```rust
// 在整个栏上注册一个「点击 + 拖动」矩形，但不占用版面
let drag = ui.interact(ui.max_rect(), ui.scope_id().with("titlebar-drag"),
                       egui::Sense::click_and_drag());
if drag.drag_started() {
    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
}
if drag.double_clicked() {
    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
}
```

命中优先级规则（egui 0.37 `hit_test.rs`）：同一位置多个控件时，**后注册者赢**（`find_closest_within` 的 tie-break 是"取最后一个 = 最上面"）。所以拖动区先注册、按钮后注册，菜单和按钮天然拿到点击，空白处才落到拖动区。

### 2.5 窗口按钮 = 纯 egui 控件

```rust
let click = |ui: &mut egui::Ui, glyph: &str, name: &str| {
    form::icon_button(ui, egui::Button::new(glyph), name, name).clicked()
};
if click(ui, "×", &self.t("btn.close", "關閉")) {
    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);   // 走未存档先询问的既有流程
}
if click(ui, if maximized { "❒" } else { "□" }, …) {
    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
}
if click(ui, "─", …) {
    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
}
```

`maximized` 从 `ui.ctx().input(|i| i.viewport().maximized)` 读取，按钮字形随状态切换。

### 2.6 原生层只做边缘缩放

```rust
// src/app/titlebar.rs
install(hwnd);                       // SetWindowSubclass(hwnd, procedure, SUBCLASS_ID, 0)，用 static Mutex 保证幂等
WM_NCHITTEST => handle_hit_test(…)   // 非最大化时，8 物理像素边框 → HTLEFT..HTBOTTOMRIGHT
其他消息 ⇒ DefSubclassProc          // 一律透传；WM_NCDESTROY 时 RemoveWindowSubclass
```

- `IsZoomed(hwnd)` 为真时**不**返回边框代码（最大化窗口不该拖边框改大小）。
- 边宽用**物理像素**固定值 8，比"逻辑像素 × 缩放"更稳（双击边框这类系统行为自己会处理 DPI）。
- 子类安装是幂等的（`static INSTALLED: Mutex<bool>`），因为 `title_bar` 每帧都可能被调用一次。

### 2.7 图标

```rust
fn load_app_icon(ctx: &egui::Context) -> TextureHandle {
    let img = image::load_from_memory(include_bytes!("../../assets/madedit-icon.png")).expect("app icon decodes");
    let size = [img.width() as usize, img.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, &img.to_rgba8());
    ctx.load_texture("madedit-app-icon", image, egui::TextureOptions::default())
}
```

懒加载一次后存在 `App` 字段里；每帧 `allocate_exact_size(vec2(icon_size, icon_size), Sense::hover())` 再 `painter().image()`，`icon_size = (bar_height * 0.5).clamp(16.0, 32.0)`。

## 3. 踩过的坑（按踩坑顺序）

### 3.1 `ui.allocate_rect` 会推进游标 —— 菜单整行被顶出面板

现象：菜单按钮画在 `y = 29.5..49.5`，面板只有 24 高；点击全部落空（hit test 拿到的是裁剪后的空矩形），但"看起来"菜单位置只是偏下一点。

根因：`allocate_rect` 会 `advance_cursor_after_rect()`，把整行内容推到下一行。

结论：**覆盖整栏的拖动/命中区一律用 `ui.interact(rect, id, sense)`**，它只注册交互矩形、不移动游标。

### 3.2 `ui.set_height()` 不改变面板的保留高度

现象：面板里 `set_height(30)`，内容确实按 30 排版，但面板本身只占 24 高，工具列从 26 开始把它盖住；面板内容溢出部分被**裁剪且不上报**给父级（egui 注释原文：*"If we do, we do so by clipping the contents, without reporting that extra size to the parent!"*）。

根因：`Panel` 的外高来自 `PanelState`（上一帧存下来的 `outer_rect`），`ui.set_height` 只影响内容 ui 的 `min_size`。

结论：**要固定高度就用 `Panel::...exact_size(h)`**；`ui.set_height` 只适合在已有面板内约束内容。

### 3.3 同名面板创建了两次

现象：高度怎么调都差 4~6px，菜单列下缘被工具列遮住一点，改公式、改内距都不对——因为调用处 `Panel::top("titlebar").show(ui, |ui| self.title_bar(ui, frame))` 里面又建了一个同名面板，外层按"上一帧内容"给自己留高度，内层再自己排，两层互相覆盖。

结论：**一个面板只在一处创建**。`title_bar()` 内部建面板，调用处就只调用函数；出现"怎么算都不对的高度差"时，第一件事是搜一遍 `Panel::` 有没有重复建。

### 3.4 用"测量已布局高度"做 ×1.5 → 逐帧放大

现象：栏高每帧涨一点（85 → 103 → 124 → …），按钮最后被排到 `y = 6398`（屏幕外）。

根因：读到的是**已被居中/拉伸后的内容高度**，它约等于当前栏高，于是"栏高 = 内容 × 1.5"每帧乘 1.5，正反馈发散。

结论：高度只能由 **字体度量 + 样式常量**推导（`text_style_height`、`button_padding`、`interact_size`），不能来自上一次布局结果。若确实需要缓存测量值，必须缓存**内容自然高**（在受约束的独立 ui 里测），且不许参与自身容器的高度计算。

### 3.5 菜单悬停切换依赖 `egui::MenuBar`

现象：点开一个菜单后，鼠标移到旁边的菜单上不切换。

根因：悬停切换在 `MenuButton::ui` 里，靠 `find_menu_bar_root(ui)` 找到带 `MenuConfig{bar:true}` tag 的 `UiStack`，并在 `ui.data` 里记住"本栏当前打开的菜单"。直接把八个 `ui.menu_button` 堆在普通布局里，就没有 bar root，切换逻辑整段失效。

结论：**菜单本体必须包在 `egui::MenuBar::new().ui(ui, |ui| …)` 里**（Windows 自绘栏和非 Windows 的 `menu_bar()` 都遵守这条）。

### 3.6 原生 `HTMINBUTTON` / `HTMAXBUTTON` / `HTCLOSE` 会和 egui 抢事件

现象：窗口按钮 hover 闪烁、点击偶尔不生效。

根因：原生命中代码会让系统绘制/处理按钮的 hover 与点击，同时 egui 也在画同一个按钮并检测 hover，两边状态不同步。

结论：**按钮用纯 egui 控件**。代价：Windows 11 不再弹最大化按钮的 Snap Layouts 浮层（要做好这个得让原生接管按钮并放弃 egui 绘制，属于另一套设计，本文不做）。

### 3.7 `MenuBar` 内部会 `set_min_size(available_width, …)`

现象：早期版本菜单栏把整行宽度吃掉，拖动区/后续控件被排到很远处。

根因：`MenuBar::ui` 里 `ui.set_min_size(vec2(ui.available_width(), height))`，故意占满整行。

结论：知道就好——它只在"同一行后面还有控件"时才会有影响；本实现里菜单是行内最后一个元素，且命中优先级由注册顺序决定，所以无害。若要在菜单右侧再放控件，请把控件放在**外层 RTL 布局**里（如 §2.3），不要指望挤在 MenuBar 内部。

### 3.8 `scope` 继承父布局方向

现象：图标跑到最右边、菜单顺序变成"说明…文件"。

根因：`ui.scope()` 出来的子 ui 用父 ui 的 layout（外层是 `right_to_left`）。

结论：**左右分区各自显式 `with_layout`**，不要依赖 `scope` 的继承方向。

### 3.9 kittest/测试相关（真实窗口，容易互相污染）

- 测试里的 kittest 窗口是**真实 Win32 窗口**，会跟用户桌面抢焦点。测试必须设独立的 `MADEDIT_CONFIG_DIR`，否则会读到/覆盖日常配置。
- 调试实例残留会锁住 `target\debug\deps\MadEdit-*.exe`，下一次 `cargo test` 直接 `LNK1104: cannot open file`。跑测试前先 `Get-Process *MadEdit* | Stop-Process`。
- 自绘标题栏带来了**重复的可访问名称**：标题栏有"关闭"，输出面板的 × 也叫"关闭"。旧测试 `get_by_label("關閉")` 会报 "two or more nodes"，要按矩形范围（如 `n.raw_bounds().y0 > 100.0`）或 `dialog(&h)` 作用域收窄。
- `test_every_dialog_keeps_its_buttons_inside_its_window` 会扫描所有按钮，标题栏按钮必须排除（按 `rect.top() < 34.0` 跳过）。
- 截图 harness（`cargo test shot -- --ignored`）跟随**系统语言**：本机系统是简体中文时菜单标签是"查看(V)"而不是"檢視(V)"，`MADEDIT_SHOT_CLICK` 要用当前语言的标签。

### 3.10 别把既有问题当成本功能引入

`with_decorations(false)` 之后，任务栏/原生窗口标题变成 "M"（主窗口标题在别处被重置，`SetWindowTextW` 也压不住）。已验证**改动前的 HEAD 也一样**，与本功能无关；判断"是不是我弄坏的"要用 `git stash` 对比一次再下结论。

### 3.11 DPI 与坐标

- `WM_NCHITTEST` 的 `lparam` 是**屏幕坐标、物理像素**，低 16 位 x、高 16 位 y（可能为负，要按 16 位有符号解包）；换算到窗口坐标要用 `GetWindowRect`（含非客户区），比较用 `GetClientRect` 之外的整窗尺寸。
- 用 DPI-unaware 的 PowerShell/C# 脚本探测命中码时，坐标会被系统虚拟化（本机 150% 时 ×1.5），别拿它去验证逻辑像素的边框位置。

## 4. 验证清单

自动化（`cargo test --locked`，均使用真实窗口与 `with_pixels_per_point(1.0)`）：

| 测试 | 保证什么 |
| --- | --- |
| `app::ui_tests::test_title_bar_contents_stay_above_the_toolbar` | 标题栏内容下缘严格高于工具列（挡住 §3.2/3.3 的回归） |
| `app::ui_tests::test_menu_bar_hover_switches_the_open_menu` | 点开"檢視"后悬停"說明"会切换（§3.5） |
| `app::ui_tests::test_short_menus_keep_the_shared_minimum_width` 等菜单系列 | 弹窗能打开、宽度/位置/滚动正常（§3.1 的直接回归） |
| `app::ui_tests::test_every_dialog_keeps_its_buttons_inside_its_window` | 新增的标题栏按钮没有跑出窗口 |

截图：`MADEDIT_SHOT_OUT=target/titlebar.png MADEDIT_SHOT_CLICK="查看(V)" cargo test --locked shot -- --ignored`（标签用系统语言）。

手工检查：拖动标题栏移动窗口；双击标题栏最大化/还原；最大化后再双击还原；四边/四角缩放；未最大化时拖到屏幕边缘 Aero Snap；菜单点开后横向滑过其它菜单；右侧 ─ □ × 的 hover 与点击；缩小窗口宽度到 340 以下时快捷控件应消失（让位给菜单）。

## 5. 以后要扩展时

- **加一个标题栏按钮**：照抄 §2.5 的 `click` 闭包，用 `form::icon_button` 并给**翻译后的动作名**（AccessKit 名称），别只放符号。
- **加别的控件**（例如搜索框）：放进外层 `right_to_left` 区（会贴右），或在内层 LTR 区放在菜单后面；注意它要**在拖动区之后注册**才拿得到点击。
- **改高度**：只改 `bar_height` 的推导式；不要引入"测量布局"。
- **想恢复 Windows 11 Snap Layouts**：需要让原生层在最大化按钮矩形上回 `HTMAXBUTTON`，并让 egui **不再绘制**该按钮的 hover 背景（否则又是 §3.6 的抢事件）。属于新设计，改之前先把 §3.6 的结论读一遍。
- **要移植到其它窗口**（比如弹窗式面板）：`Panel::top` 换成 `Window`/`Area`，`exact_size` 的逻辑换成外层 `Frame`，拖动区与命中优先级的规则不变。

## 6. 相关文件

| 路径 | 内容 |
| --- | --- |
| [`src/main.rs`](../src/main.rs) | `with_decorations(false)` |
| [`src/app/mod.rs`](../src/app/mod.rs) | `App::title_bar`：面板、布局、拖动区、窗口按钮、图标 |
| [`src/app/menus.rs`](../src/app/menus.rs) | `menus()`（八个菜单本体）、`menu_bar()`（非 Windows）、`quick_controls()` |
| [`src/app/titlebar.rs`](../src/app/titlebar.rs) | Win32 子类 + `WM_NCHITTEST` 边缘缩放 |
| [`src/app/ui_tests.rs`](../src/app/ui_tests.rs) | 标题栏/菜单回归测试 |
| [`docs/mixed-dpi-drag-fix.md`](./mixed-dpi-drag-fix.md) | 同批修复的混合 DPI 拖动问题（winit 补丁） |
