# egui 多行输入框在自适应窗口里的坑与固定模式（备忘）

查找和替换浮窗位于**自适应高度的 egui 窗口**中，需要一个**固定高度的多行输入框**
（4 行、内部滚动、UE 风格）。egui 在这件事上非常坑：`TextEdit::multiline`
**会随内容自动长高**，而且内容矩形、视口矩形、窗体矩形三个矩形同时影响布局、
绘制和滚动，稍不注意就全乱。本备忘记录我们实际踩过的坑和最终可行的结构。

**参考实现已经封装成通用控件**，新代码不要再手搭这套结构：

- 容器与描边：`form::scrolling_text_edit` / `form::field_border`（`src/app/form.rs`）；
- 查找浮窗专用的按键与滚轮行为：`multiline_field`（`src/app/find.rs`）。

## 实际踩过的坑

| 症状 | 根因 |
| --- | --- |
| 输入框随粘贴/输入的内容变高 | `TextEdit::multiline` 会自适应内容高度，没有任何东西约束它。 |
| 聚焦（白框）或滚动时，边框"撑破"输入框 | 边框画在了 TextEdit 的**内容矩形**上（即长高后的全高），而不是视口矩形。 |
| 滚动条切过输入框边框 | 滚动条由 `ScrollArea` 画在视口边缘；视口若与描边框重合，滚动条就会压住圆角。 |
| 鼠标滚轮滚动时，红色错误边框盖住标题栏 | 错误描边锚定在**内容矩形**上，滚动时内容矩形坐标整体上移。 |
| 状态行（"共 N 个符合"）忽隐忽现、布局上下跳 | 状态行只在有文字时才占行，出现/消失会推挤布局一行。 |
| 所有控件叠在一起 | `Ui::new_child` **不会推进父级光标**；必须先用 `allocate_exact_size` 占位，否则后续控件全部画在原点。 |
| 输入光标又高又粗，比文字高出一截 | egui 的 `TextEdit` 光标 = **行高再上下各加 1.5 点**、2 点宽。行高本身含字体的 ascent/descent，13px 字号下行高 15、光标 18px，比 CJK 字形高约 1/3。 |
| Ctrl+Enter 插入换行后，滚动条不跟随，光标跑出视口 | 换行是在输入框**绘制之后手动插入**的。egui 只在它自己处理按键、改动了文本或选区时才把光标卷入视口；外部改动在下一帧"没有变化"，视口就留在原地。 |
| 替换框按 Enter，被替换的却是下一个匹配 | 两个输入框都读全局 `key_pressed(Enter)`，不分焦点；收尾时又先执行"查找下一个"再替换。 |

## 可用的固定模式（已封装）

```rust
// 容器 + 输入框 + 描边，见 src/app/form.rs
let id = ui.scope_id().with("my-field"); // 局部唯一 id（egui 0.37 已弃用 Id::new / Ui::id）
let edit = form::scrolling_text_edit(ui, "my-field", id, &mut text, "提示文字", 4, return_key);
form::field_border(ui, &edit, invalid);
let resp = edit.response; // 焦点、changed()、request_focus() 都在它上面
// edit.rect 是带边框的视口矩形：自动化矩形断言、额外装饰都锚它
```

`return_key` 是"哪个组合键插入换行"（`Option<KeyboardShortcut>`），见下一节。

控件内部做了什么（要改它，先读完这几条）：

1. **钉死容器高度**：`正文行高 × 行数 + 上下内边距（4×2）+ 16`，宽度取
   `ui.max_rect().width()`。
2. **先铺底色**：TextEdit 自身既无框也无底色。
3. **占位推进光标**：`allocate_exact_size`。`Ui::new_child` 不推进父级光标，
   不占位的话，后续所有控件会叠在输入框上。
4. **四周内缩 (6, 4) 的子 `Ui` 里放 `ScrollArea`**：`auto_shrink([false, false])`，
   `min_scrolled_height` 与 `max_height` 必须成对设成内缩后的高度
   （视口高度 = max_height，绝不随内容）。
5. **无框 `TextEdit`**：`.id(id).return_key(return_key).frame(Frame::NONE)
   .desired_width(INFINITY)`。它自带的边框属于滚动内容，会破版。
6. **描边画在视口矩形上**（`form::field_border`）：聚焦 = active，悬停 = hovered，
   否则 noninteractive；`invalid` 为真时红色。绝不能画在 `resp.rect` 上——
   那是未裁剪的内容矩形，会随内容长高、随滚动位移。
7. **光标自绘**：egui 的光标高度没有样式项可调。把控件内 `text_cursor.stroke`
   设为透明，再按 `TextEdit::show()` 返回的 `galley_pos` 与光标位置画一根
   **与行等高、1.5 点宽**的线（闪烁沿用样式的节奏，时间自己记）。
   **输入法合成期间必须保留 egui 自己的光标**：Windows 输入法给的是"空活动区间"，
   合成期 egui 会在活动位置画光标。egui 不公开合成状态，只能从 `Event::Ime` 跟踪
   （非空 `Preedit` 开始，空 `Preedit`／`Commit` 结束）。

历史下拉箭头和错误描边的锚定方式相同：锚 `edit.rect`（视口），绝不锚 `resp.rect`（内容）。

## Enter 键行为（Windows 习惯）

`TextEdit::multiline` 默认**纯 Enter 插入 `\n`**。查找框要的是反过来：纯 Enter
触发查找／替换，Ctrl+Enter 才换行。做法是把换行**交给输入框自己**
（`find.rs` 的 `multiline_field`）：

- 把 `form::scrolling_text_edit` 的 `return_key` 设成 `Ctrl+Enter`
  （`egui::KeyboardShortcut`）。命中时由 egui 自己的事件处理插入 `\n`：同帧更新
  排版、移动光标，并 `scroll_to_rect` 把光标卷入视口。
- **不要**在绘制之后手动改文本和光标状态来"补一个换行"。egui 只在它自己处理
  按键、改动了文本或选区时才滚动（`text_edit/builder.rs`：
  `response.changed() || selection_changed`），外部改动在下一帧"没有变化"，视口
  就留在原地——光标跑到视口外，用户看不到插入位置。
- 纯 Enter、Shift+Enter 不匹配 `return_key`，不会插入，所以也**不需要**"先克隆
  文本、再还原"的补丁（那套做法每帧还要多拷一份文本）。
- **只有持有焦点的框回应 Enter**（`resp.has_focus()`）。替换页两个框同时显示，
  必须各管各的按键：都读全局 `key_pressed(Enter)` 曾让替换框的 Enter 先"查找下
  一个"再替换，跳过当前匹配。修饰键的判断要与 `return_key` 一致
  （`modifiers.matches_logically(Modifiers::CTRL)`），插入换行的那一下不算"纯 Enter"。
- 若确需操作光标状态：`TextEdit::load_state` → `state.cursor.char_range()`，
  注意 `CCursor.index` 是**字符偏移**（`CharIndex(pub usize)`），不是字节偏移。
- **Esc 让位于弹层**：egui 的 `Popup` 在自己的 `show()` 里处理 Esc 并回写
  开关旗标，帧末再查旗标已经是 false。要在绘制前记下"弹层是否打开"，
  据此决定这一下 Esc 归弹层还是归窗口。

## 已知取舍

- 滚轮会移动光标（按滚动量整行步进，`smooth_scroll_delta ÷ 行高`），而不是
  纯视图滚动。egui 会对 ScrollArea 内的焦点控件"逃逸裁剪"以保证光标可见，
  想实现纯视图滚动需要深改 TextEdit 内部，性价比低。对查找框来说，光标只
  决定下一次"查找下一个"的起点，可以接受。
- 粘贴大段文本的瞬间，可能有单帧的高度闪烁（egui 的焦点裁剪逃逸在布局
  稳定前生效）。
- 内层 `TextEdit` 的 AccessKit 节点矩形报告的是**未裁剪的内容尺寸**，
  自动化矩形断言必须量滚动容器（`edit.rect`），不能量文本节点。
- 字号是固定的 12.5 pt（`find.rs` 的 `SMALL_TEXT`），不随正文字号缩放；
  改成相对字号需要重新目视核对版面。

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
- 整段替换上百行时不要手抄 `old_string`，容易差一个字符。改用按字节读写、
  **带锚点校验**的脚本：先核对首尾锚点行，对不上就中止不写。本仓库工作区
  是 CRLF（`core.autocrlf=true`），用 `write` 整文件写出的 LF 要转回 CRLF。
