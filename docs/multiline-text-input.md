# egui 多行输入框在自适应窗口里的坑与固定模式（备忘）

查找和替换浮窗位于**自适应高度的 egui 窗口**中，需要一个**固定高度的多行输入框**
（4 行、内部滚动、UE 风格）。egui 在这件事上非常坑：`TextEdit::multiline`
**会随内容自动长高**，而且内容矩形、视口矩形、窗体矩形三个矩形同时影响布局、
绘制和滚动，稍不注意就全乱。本备忘记录我们实际踩过的坑和最终可行的结构，
避免下次再试错。参考实现：`src/app/find.rs`（`find_bar`）。

## 实际踩过的坑

| 症状 | 根因 |
| --- | --- |
| 输入框随粘贴/输入的内容变高 | `TextEdit::multiline` 会自适应内容高度，没有任何东西约束它。 |
| 聚焦（白框）或滚动时，边框"撑破"输入框 | 边框画在了 TextEdit 的**内容矩形**上（即长高后的全高），而不是视口矩形。 |
| 滚动条切过输入框边框 | 滚动条由 `ScrollArea` 画在视口边缘；视口若与描边框重合，滚动条就会压住圆角。 |
| 鼠标滚轮滚动时，红色错误边框盖住标题栏 | 错误描边锚定在**内容矩形**上，滚动时内容矩形坐标整体上移。 |
| 状态行（"共 N 个符合"）忽隐忽现、布局上下跳 | 状态行只在有文字时才占行，出现/消失会推挤布局一行。 |
| 所有控件叠在一起 | `Ui::new_child` **不会推进父级光标**；必须先用 `allocate_exact_size` 占位，否则后续控件全部画在原点。 |

## 可用的固定模式

```rust
// 1. 先钉死固定高度的容器矩形
let box_h = ui.text_style_height(&egui::TextStyle::Body) * 4.0 + 24.0; // 4 行 + TextEdit 边距
let vp_rect = egui::Rect::from_min_size(
    egui::pos2(ui.max_rect().left(), ui.cursor().top()),
    egui::vec2(ui.max_rect().width(), box_h),
);

// 2. 先铺底色（TextEdit 已无自身边框/底色）
ui.painter().rect_filled(vp_rect, 6.0, ui.visuals().extreme_bg_color);

// 3. 占位推进光标：new_child 不推进父级光标，
//    不占位的话，后续所有控件会叠在输入框上
ui.allocate_exact_size(egui::vec2(vp_rect.width(), box_h), egui::Sense::hover());

// 4. 无框 TextEdit（它自带的边框属于滚动内容，会破版）；
//    滚动视口四周内缩，让滚动条不触碰圆角边框
let field = egui::TextEdit::multiline(&mut text)
    .desired_width(vp_rect.width() - 12.0) // 给滚动条留空间
    .desired_rows(4)
    .frame(egui::Frame::NONE)
    .margin(egui::Margin::symmetric(6, 4));
let mut scroll_ui = ui.new_child(
    egui::UiBuilder::new()
        .id_salt("some-scroll-area")
        .max_rect(vp_rect.shrink2(egui::vec2(6.0, 6.0)))
        .layout(egui::Layout::top_down(egui::Align::LEFT)),
);
let resp = egui::ScrollArea::vertical()
    .id_salt("some-scroll")
    .auto_shrink([false, false]) // 视口高度 = max_height，绝不随内容
    .min_scrolled_height(box_h - 12.0) // min/max 必须成对设置
    .max_height(box_h - 12.0)
    .show(&mut scroll_ui, |ui| ui.add(field))
    .inner;

// 5. 边框画在「视口矩形」上（绝不能画在 resp.rect 上——
//    那是未裁剪的内容矩形，会随内容长高、随滚动位移）
let focused = resp.has_focus();
let hovered = ui.rect_contains_pointer(vp_rect);
let border = if focused {
    ui.visuals().widgets.active.bg_stroke
} else if hovered {
    ui.visuals().widgets.hovered.bg_stroke
} else {
    ui.visuals().widgets.noninteractive.bg_stroke
};
ui.painter().rect_stroke(vp_rect, 6.0, border, egui::StrokeKind::Inside);
```

历史下拉箭头和错误描边的锚定方式相同：锚 `vp_rect`（视口），
绝不锚 `resp.rect`（内容）。

## Enter 键行为（Windows 习惯）

`TextEdit::multiline` 在**纯 Enter 时插入 `\n`，带修饰键（Ctrl）时不插入**。
搜索框要的是反过来：

- 纯 Enter：把文本还原为进帧前的值（撤销刚插入的 `\n`），并触发动作（查找下一个）。
- Shift+Enter：同上，但触发"查找上一个"。
- Ctrl+Enter：egui 什么都没插入，需要手动在光标处插入 `\n`
  （`TextEdit::load_state` → 读 `state.cursor.char_range()` → 插入字符串 →
  `set_char_range` + `store`）。注意 `CCursor.index` 是**字符偏移**
  （`CharIndex(pub usize)`），不是字节偏移。

## 已知取舍

- 滚轮滚动会移动光标（每格 ±3 行），而不是纯视图滚动。egui 会对
  ScrollArea 内的焦点控件"逃逸裁剪"以保证光标可见，想实现纯视图滚动
  需要深改 TextEdit 内部，性价比低。对查找框来说，光标只决定下一次
  "查找下一个"的起点，可以接受。
- 粘贴大段文本的瞬间，可能有单帧的高度闪烁（egui 的焦点裁剪逃逸
  在布局稳定前生效）。
- 内层 `TextEdit` 的 AccessKit 节点矩形报告的是**未裁剪的内容尺寸**，
  自动化矩形断言必须量滚动容器，不能量文本节点。

## 调试经验

- `ScrollArea` 内层响应的 `resp.rect` 是**内容矩形**（随内容长高）；
  视口高度才是 `min/max_scrolled_height`。所谓"框撑大了"，本质就是
  把装饰画在了内容矩形上。
- 一定要量化，不要目测：一个打印控件矩形的 egui_kittest 测试
  （`query_all_by(...).next().rect()`）一次就找到了光标不推进导致的
  控件重叠，而此前两轮看截图猜参数全部失败。
- 绝对不要用 PowerShell 文本管道（`Get-Content` / `Set-Content`）改
  源码文件：Windows PowerShell 5.1 会把 UTF-8 按 ANSI 解码，静默毁掉
  中文注释。改文件用编辑工具，恢复用 `git show HEAD:<path>`。
