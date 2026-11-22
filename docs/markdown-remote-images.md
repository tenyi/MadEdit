# egui Markdown 遠端／本地圖片渲染：實現思路與踩坑指南

> 本文件記錄 MadEdit 為 Markdown 預覽加入「遠端圖片（含 SVG 徽章）與 HTML `<img>` 標籤渲染」的
> 完整實現過程，供移植到其他 egui 專案時參考。
> 對應提交：`bd84d09`（遠端圖片基礎能力）、`0c55ee6`（HTML img／GIF／背景點陣化）。
>
> 環境：egui/eframe **0.37** + egui_commonmark **0.26** + egui_extras **0.37**（Windows 桌面）。

## 一、依賴特性（坑 1：預設什麼都沒有）

```toml
egui_commonmark = { version = "0.26", default-features = false,
                    features = ["pulldown_cmark", "load-images", "fetch"] }
egui_extras     = { version = "0.37", default-features = false,
                    features = ["svg", "http", "gif", "image"] }
```

- **`http`／`fetch` 不開 = 遠端圖片直接報 `No matching BytesLoader`**。
  egui 沒有內建網路下載，靠 ehttp（`egui_extras/http`）或 egui_commonmark 的 fetch。
- `svg`：SVG 點陣化（內部用 usvg/resvg）。
- `gif`：GIF 逐幀動畫；`image`：一般點陣圖解碼。
- 渲染入口必須安裝一次 loader：

```rust
egui_extras::install_image_loaders(ctx);
```

## 二、坑點與解法

### 坑 1：SVG loader 以 URI 副檔名匹配，但不認 `?` 查詢串

egui 的 `has_extension(uri, "svg")` 只按 `#` 分割，不處理 `?`。
`badge.svg?branch=main` 的「副檔名」被判成 `svg?branch=main` → 內建 SvgLoader 拒載；
接著 image crate loader 也不支援 SVG 格式 → 最終報
`No matching ImageLoader for format: "image/svg+xml; charset=utf-8"`。

**解法**：自訂 loader，自行判斷「去掉 `?…` 後仍以 `.svg` 結尾」。

### 坑 2：egui_commonmark 把 HTML 當純文字渲染

README 常見的 `<img src="data/logo.jpeg" height=200>` 永遠不會觸發圖片載入，
只會顯示成文字（`Event::Html → event_text`，egui 不渲染 HTML）。

兩條路：

- **方案 A（MadEdit 採用）**：`CommonMarkViewer::render_html_fn(Some(&closure))` 鉤子，
  HTML 片段由自己渲染——`<img>` 依 `height` 屬性等比顯示（loader 端用
  `SizeHint::Height` 點陣化），其餘標籤剝掉當文字。
- 方案 B：把 `<img>` 轉成 Markdown 圖片語法 `![alt](uri)`（簡單，但丟失 height/align）。

注意 pulldown_cmark 會把 `<p>`／`<img>`／`</p>` 拆成**多個** HTML 事件，
包裝標籤（`<p>`／`<br>`）要單獨清理，否則會以字面文字殘留。

### 坑 3：相對路徑要自己按 `.md` 所在目錄解析

Markdown 語法圖（`![x](data/a.png)`）與 HTML `<img>` 都要：
`base_dir.join(src)` → `file:///` URI。

做法：解析階段（pulldown 事件遍歷）統一登記解析後的 URI 清單——
錯誤提示（⚠ + URI）與「重新載入圖片」功能都依賴這份清單。

### 坑 4：巨型 SVG 凍結 UI 數分鐘（最大的坑）

實測打包記錄 SVG 原生尺寸高達 `66437×22000`，resvg 按原生尺寸渲染：
單張 1～124 秒，10 張 = 約 400 秒的 UI 凍結。

**解法**：渲染前先 `usvg::Tree::from_data` 解析自然尺寸，把渲染目標等比壓到
`ctx.max_texture_side`（預設 2048）內，以 `SizeHint::Scale(壓縮後倍率)` 渲染——
毫秒級完成；顯示清晰度由 egui 依版面寬度縮放，無損。

### 坑 5：超過紋理上限直接崩潰（不是報錯）

egui 貼紋理前斷言邊長 ≤ `max_texture_side`（2048），超限 **panic**。
所有點陣化結果過一道 `clamp_texture_size`（等比縮小）雙保險。

### 坑 6：同步點陣化阻塞 UI 執行緒

即使限制了尺寸，多張大 SVG 逐個點陣化仍會卡 UI。
照 egui_extras 內部 `DecodeCache` 的模式自己做：

1. 快取插入 `Poll::Pending`（先查重，避免多幀重複排程）；
2. `std::thread::spawn` 背景「解析→點陣化→寫回 `Poll::Ready`」；
3. 寫回後呼叫 `ctx.request_repaint()`（**不要在鎖內呼叫 ctx**，會死鎖——
   egui 官方 DecodeCache 註解明確警告過這個死鎖場景）；
4. 期間 UI 顯示佔位框，介面保持流暢。

注意 egui_extras 的 `DecodeCache` 是 `pub(crate)`，外部用不了，要自己複製這個模式。

### 坑 7：`SizeHint::default()` = `Scale(1.0)`，會按原生尺寸渲染

呼叫端已給出夠小的目標（如 HTML 的 `height=200` → `SizeHint::Height(200)`）時
**直接用它**——幾十毫秒就能點陣化完；只有 `Scale` 類「按原生尺寸」的請求才做坑 4 的壓縮。
我們最初無腦壓到 2048，反而比必要的速度慢（記錄 SVG 元素極多，2048 寬也要秒級）。

### 坑 8：數值屬性解析要容忍髒尾巴

`<img height=200/>` 的屬性值是 `200/`（`/` 是自閉合的一部分）。
解析數值時只取前導數字（容忍 `px`、`/` 尾巴），雙引號／單引號／無引號三種寫法都支援。

## 三、程式碼骨架

### RemoteSvgLoader（精簡版）

```rust
pub struct RemoteSvgLoader {
    options: Arc<resvg::usvg::Options<'static>>,
    cache: Arc<Mutex<HashMap<(String, SizeHint), std::task::Poll<Rasterized>>>>,
}
type Rasterized = Result<Arc<egui::ColorImage>, String>;

impl ImageLoader for RemoteSvgLoader {
    fn id(&self) -> &str { Self::ID }

    fn load(&self, ctx: &egui::Context, uri: &str, size_hint: SizeHint) -> ImageLoadResult {
        // 1. 只接 path（去掉 ?… 與 #…）以 .svg 結尾的 http(s)/file URI
        // 2. 快取命中 Ready → 直接回傳
        // 3. ctx.try_load_bytes(uri) 取位元組；快取插 Pending（查重）後排背景執行緒：
        //    a. 解析自然尺寸，依呼叫端 hint / 紋理上限算出渲染 hint
        //    b. load_svg_bytes_with_size(&bytes, hint, &options) → clamp_texture_size
        //    c. 寫回 Ready → ctx.request_repaint()（鎖外）
        // 4. 回 Ok(ImagePoll::Pending { size: None })，UI 顯示佔位框
    }

    fn forget(&self, uri: &str) { /* 快取清除，配合 ctx.forget_image */ }
    fn forget_all(&self) { /* 同上 */ }
    fn byte_size(&self) -> usize { 0 /* 背景點陣化，簡化回報 */ }
}
```

完整實作見 `src/app/mdpreview.rs` 的 `RemoteSvgLoader`（含 `clamp_texture_size`）。

### HTML `<img>` 渲染關鍵（render_html_fragment，約 40 行）

```rust
let html = move |ui: &mut egui::Ui, html: &str| render_html_fragment(ui, html, base_dir);
let viewer = CommonMarkViewer::new()
    .explicit_image_uri_scheme(true)
    .render_math_fn(Some(&math))
    .render_html_fn(Some(&html));   // ← HTML 接管

// render_html_fragment 內：
// <img> 提取 src / height（html_attr + html_numeric_attr）→
// 相對路徑 → image_uri(…) 解析成 file:/// 絕對路徑
// SizeHint::Height(h) → ctx.try_load_texture → Ready 時
// Image::from_texture(texture).fit_to_exact_size(vec2(w, h))（等比）
// Pending → 佔位框；Err → ⚠ + URI
// 其餘標籤剝除，剩餘文字 ui.label()
```

## 四、驗證方法（無頭、可進 CI）

- **Loader 單測**：`ctx.include_bytes(uri, svg_bytes)` 餵本地位元組 +
  `try_load_image` **輪詢**等 `Ready`（背景點陣化是異步的）；
  超大 SVG（如 4000×3000）斷言結果 ≤2048 且保持寬高比。
- **路徑重寫單測**：相對路徑解析、遠端不動、程式碼區塊內的 `<img>` 不改寫。
- **端到端（kittest）**：開啟真實 README，檢查 `preview.image_uris` 清單完整、
  每幀耗時正常（首幀應在秒級內，不能出現分鐘級凍結）。

## 五、最終效果

- 遠端 SVG 徽章（GitHub Actions、Zenodo，帶查詢串）正常顯示；
- README 內 `<img>` 本地相對圖片按 `height` 屬性分層顯示，與 VSCode 觀感一致；
- GIF 自動播放動畫；
- 大 SVG 不再凍結（首幀從 400 秒降到 3 秒內，之後由快取秒開）、不再崩潰。
