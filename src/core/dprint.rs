//! dprint 外部格式化整合
//!
//! 「帶 DPRINT」發行版會在執行檔旁附帶固定版本的 dprint 執行檔與 Wasm 插件
//! （`assets/formatters/manifest.json` 為唯一版本與雜湊來源）；乾淨版沒有這些
//! 檔案，此時退回到 PATH 上的 `dprint`（設定檔仍使用隨附或開發目錄中的
//! `dprint.json`）。兩者都不存在時，指令會顯示安裝提示而不更動內容。
//!
//! 呼叫方式與 AStyle 相同：把整份文件寫進 stdin，讀回 stdout。插件以副檔名
//! 選擇（`LANGUAGES`），未支援的格式直接拒絕，避免 dprint 靜默原樣返回
//! 造成「成功但沒效果」的假象。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::document::Document;

/// 各插件支援的副檔名（與 assets/formatters/dprint.json 的插件一致；先到先得）
pub const LANGUAGES: &[(&str, &[&str])] = &[
    ("clang-format", &["c", "cc", "cpp", "cxx", "h", "hh", "hpp", "hxx", "m", "mm"]),
    ("typescript", &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"]),
    ("json", &["json", "jsonc"]),
    ("malva", &["css", "scss", "sass", "less"]),
    ("markup", &["html", "vue", "svelte", "astro", "xml", "svg", "jinja", "jinja2", "j2", "twig", "njk", "vto", "mustache", "hbs", "handlebars", "wsdl", "xsd", "xslt", "xsl"]),
    ("yaml", &["yaml", "yml"]),
    ("toml", &["toml"]),
    ("markdown", &["md", "markdown"]),
    ("graphql", &["graphql", "gql"]),
    ("sql", &["sql"]),
    ("dockerfile", &["dockerfile"]),
    ("shfmt", &["sh", "bash", "zsh", "mksh", "bats"]),
    ("ruff", &["py"]),
    ("gofumpt", &["go"]),
    ("mago", &["php"]),
    ("stylua", &["lua", "luau"]),
    // csharpier 也收 xml，但 markup 排在前面（dprint.json 的插件順序相同），.xml 由 markup 接手
    ("csharpier", &["cs", "csx", "csproj", "props", "targets", "slnx", "config", "xaml", "axaml"]),
];

/// 無副檔名（或點開頭）檔案的對應，與 dprint 的 fileNames 概念相同
pub const FILE_NAMES: &[(&str, &str)] = &[
    ("Dockerfile", "dockerfile"),
    (".bashrc", "shfmt"),
    (".bash_profile", "shfmt"),
    (".bash_logout", "shfmt"),
    (".zshenv", "shfmt"),
    (".zprofile", "shfmt"),
    (".zshrc", "shfmt"),
    (".zlogin", "shfmt"),
    (".zlogout", "shfmt"),
];

/// 格式化大型文件（含首次插件編譯）的上限
const TIMEOUT: Duration = Duration::from_secs(60);

pub fn plugin_for_extension(ext: &str) -> Option<&'static str> {
    let ext = ext.to_ascii_lowercase();
    LANGUAGES.iter().find(|(_, exts)| exts.contains(&ext.as_str())).map(|(plugin, _)| *plugin)
}

pub fn plugin_for_file_name(name: &str) -> Option<&'static str> {
    FILE_NAMES.iter().find(|(file, _)| *file == name).map(|(_, plugin)| *plugin)
}

/// 沒有檔名時，依目前語法名稱推回代表副檔名（對應 syntax.rs 的 syntect 名稱）
pub fn extension_for_syntax(syntax: &str) -> Option<&'static str> {
    match syntax.trim() {
        "C" => Some("c"),
        "C++" => Some("cpp"),
        "Objective-C" => Some("m"),
        "JavaScript" => Some("js"),
        "JSON" => Some("json"),
        // TOML 檔案也使用 YAML 語法；未存檔時以更常用的 YAML 為準（存檔後走副檔名）
        "YAML" => Some("yaml"),
        "Markdown" => Some("md"),
        "Python" => Some("py"),
        "CSS" => Some("css"),
        "SCSS" => Some("scss"),
        "HTML" => Some("html"),
        "XML" => Some("xml"),
        "SQL" => Some("sql"),
        "PHP" => Some("php"),
        "Bourne Again Shell (bash)" => Some("sh"),
        _ => None,
    }
}

/// 傳給 `dprint fmt --stdin` 的檔名：優先目前檔名，未存檔時用語法推導
pub fn stdin_file_name(doc: &Document) -> Result<String, String> {
    if let Some(path) = &doc.path && let Some(name) = path.file_name() {
        return Ok(name.to_string_lossy().into_owned());
    }
    let ext = extension_for_syntax(&doc.syntax)
        .ok_or_else(|| "無法判斷檔案類型：請先存檔，或把語法切到支援的語言（C/C++、JS/TS、JSON、Markdown、TOML、Python）。".to_string())?;
    Ok(format!("Untitled.{ext}"))
}

/// 可能存放隨附 formatter（dprint + dprint.json + plugins/）的資料夾，
/// 與拼字字典相同的多重位置策略：macOS App Resources、執行檔旁、開發目錄。
pub fn formatter_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        dirs.push(exe_dir.join("../Resources/formatters"));
        dirs.push(exe_dir.join("formatters"));
    }
    dirs.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/formatters")));
    dirs
}

fn helper_name() -> &'static str {
    if cfg!(windows) { "dprint.exe" } else { "dprint" }
}

/// 找出可用的 dprint：隨附版（macOS Helper 布局與可攜布局）優先，再退回 PATH。
/// 回傳（執行檔, 設定檔）。
pub fn find_helper() -> Option<(PathBuf, PathBuf)> {
    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        // macOS App：Contents/Helpers/dprint + Contents/Resources/formatters/dprint.json
        let helper = exe_dir.join("../Helpers").join(helper_name());
        let config = exe_dir.join("../Resources/formatters/dprint.json");
        if helper.is_file() && config.is_file() {
            return Some((helper, config));
        }
    }
    for dir in formatter_dirs() {
        let helper = dir.join(helper_name());
        let config = dir.join("dprint.json");
        if helper.is_file() && config.is_file() {
            return Some((helper, config));
        }
    }
    // 乾淨版：使用者自裝的 dprint（PATH），設定檔仍用隨附／開發目錄的那份
    if let Ok(output) = Command::new("dprint").arg("--version").stdin(Stdio::null()).output() && output.status.success() {
        let config = formatter_dirs().into_iter().map(|d| d.join("dprint.json")).find(|p| p.is_file())?;
        return Some((PathBuf::from("dprint"), config));
    }
    None
}

fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("MadEdit").join("dprint"))
}

/// 以 dprint 格式化整份文字。`file_name` 決定使用哪支插件與語言規則。
pub fn format_text(text: &str, file_name: &str) -> Result<String, String> {
    let path = Path::new(file_name);
    let plugin = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(plugin_for_extension)
        .or_else(|| path.file_name().and_then(|n| n.to_str()).and_then(plugin_for_file_name))
        .ok_or_else(|| format!("dprint 沒有支援「{file_name}」的格式化插件（支援：{}）。", supported_summary()))?;
    let (helper, config) = find_helper().ok_or_else(|| {
        "找不到 dprint。此版本未內建格式化工具；請改用「帶 DPRINT」版本，或自行安裝 dprint 並確認它在 PATH 中。".to_string()
    })?;

    let mut cmd = Command::new(&helper);
    cmd.args(["fmt", "--config"])
        .arg(&config)
        .arg("--stdin")
        .arg(file_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cache) = cache_dir() {
        // 隨附的 App 目錄是唯讀的；插件編譯結果一律放在使用者快取
        cmd.env("DPRINT_CACHE_DIR", cache);
    }
    #[cfg(windows)]
    {
        // 不顯示主控台視窗
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("無法啟動 dprint（{}）：{e}", helper.display()))?;

    // stdin 與 stdout 都可能超過管線緩衝，必須在不同執行緒讀寫，否則會互相卡死
    let mut stdin = child.stdin.take().expect("dprint stdin");
    let input = text.to_string();
    let writer = thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let output = child.wait_with_output();
        let _ = tx.send(output);
    });
    let _ = writer.join();

    let output = rx
        .recv_timeout(TIMEOUT)
        .map_err(|_| format!("dprint 在 {} 秒內沒有完成，已放棄（插件首次編譯可能較久，可再試一次）。", TIMEOUT.as_secs()))?
        .map_err(|e| format!("讀取 dprint 輸出失敗：{e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            return Err(format!("dprint 結束代碼 {}（{} 插件）。", output.status, plugin));
        }
        return Err(format!("dprint：{stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn supported_summary() -> String {
    let mut names = Vec::new();
    for (_, exts) in LANGUAGES {
        for ext in *exts {
            if !names.iter().any(|n: &String| n == ext) {
                names.push((*ext).to_string());
            }
        }
    }
    names.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// dprint.json 設定區段的鍵名（與插件 id 不同者只有 clang-format → clangFormat）
    fn config_key(plugin: &str) -> &str {
        if plugin == "clang-format" { "clangFormat" } else { plugin }
    }

    #[test]
    fn test_plugin_for_extension_matches_clang_and_friends() {
        assert_eq!(plugin_for_extension("cpp"), Some("clang-format"));
        assert_eq!(plugin_for_extension("MM"), Some("clang-format")); // 大小寫不拘
        assert_eq!(plugin_for_extension("jsonc"), Some("json"));
        assert_eq!(plugin_for_extension("markdown"), Some("markdown"));
        assert_eq!(plugin_for_extension("py"), Some("ruff"));
        // clang 是必要支援：主要 C/C++ 副檔名都要有插件
        for ext in ["c", "h", "cc", "cpp", "cxx", "hpp"] {
            assert_eq!(plugin_for_extension(ext), Some("clang-format"), "{ext}");
        }
        // 常用語言
        assert_eq!(plugin_for_extension("css"), Some("malva"));
        assert_eq!(plugin_for_extension("less"), Some("malva"));
        assert_eq!(plugin_for_extension("html"), Some("markup"));
        assert_eq!(plugin_for_extension("SVG"), Some("markup"));
        assert_eq!(plugin_for_extension("vue"), Some("markup"));
        assert_eq!(plugin_for_extension("yml"), Some("yaml"));
        assert_eq!(plugin_for_extension("graphql"), Some("graphql"));
        assert_eq!(plugin_for_extension("sql"), Some("sql"));
        assert_eq!(plugin_for_extension("dockerfile"), Some("dockerfile"));
        assert_eq!(plugin_for_extension("zsh"), Some("shfmt"));
        assert_eq!(plugin_for_extension("go"), Some("gofumpt"));
        assert_eq!(plugin_for_extension("php"), Some("mago"));
        assert_eq!(plugin_for_extension("luau"), Some("stylua"));
        assert_eq!(plugin_for_extension("cs"), Some("csharpier"));
        // .xml 同時在 markup 與 csharpier 的範圍，先到先得交給 markup
        assert_eq!(plugin_for_extension("xml"), Some("markup"));
        // 未收錄的語言必須明確拒絕，不能靠 dprint 原樣返回
        assert_eq!(plugin_for_extension("rs"), None);
        assert_eq!(plugin_for_extension("txt"), None);
        assert_eq!(plugin_for_extension(""), None);
    }

    #[test]
    fn test_plugin_for_file_name_matches_dockerfile_and_dotfiles() {
        assert_eq!(plugin_for_file_name("Dockerfile"), Some("dockerfile"));
        assert_eq!(plugin_for_file_name(".zshrc"), Some("shfmt"));
        assert_eq!(plugin_for_file_name(".bashrc"), Some("shfmt"));
        assert_eq!(plugin_for_file_name("Makefile"), None);
    }

    #[test]
    fn test_extension_for_syntax_and_stdin_file_name() {
        assert_eq!(extension_for_syntax("C++"), Some("cpp"));
        // TOML 檔案也用 YAML 語法；未存檔時先按 YAML 處理（存檔後走副檔名）
        assert_eq!(extension_for_syntax("YAML"), Some("yaml"));
        assert_eq!(extension_for_syntax("CSS"), Some("css"));
        assert_eq!(extension_for_syntax("HTML"), Some("html"));
        assert_eq!(extension_for_syntax("Bourne Again Shell (bash)"), Some("sh"));
        assert_eq!(extension_for_syntax("Rust"), None);

        let mut doc = Document::default();
        doc.syntax = "C++".into();
        assert_eq!(stdin_file_name(&doc).unwrap(), "Untitled.cpp");

        doc.path = Some(std::env::temp_dir().join("demo.c"));
        assert_eq!(stdin_file_name(&doc).unwrap(), "demo.c");

        doc.syntax = "Plain Text".into();
        doc.path = None;
        assert!(stdin_file_name(&doc).is_err());
    }

    /// dprint.json 的插件清單必須與 LANGUAGES 一致，兩邊不同步時在測試就失敗
    #[test]
    fn test_bundled_config_lists_exactly_the_supported_plugins() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../assets/formatters/dprint.json")).expect("dprint.json 可解析");
        let plugins: Vec<&str> = config["plugins"]
            .as_array()
            .expect("plugins 陣列")
            .iter()
            .map(|p| {
                let s = p.as_str().expect("字串");
                s.strip_prefix("./plugins/")
                    .and_then(|s| s.strip_suffix(".wasm"))
                    .unwrap_or_else(|| panic!("插件路徑格式不符：{s}"))
            })
            .collect();
        let expected: Vec<&str> = LANGUAGES.iter().map(|(p, _)| *p).collect();
        let mut a = plugins.clone();
        let mut b = expected.clone();
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b, "dprint.json 與 LANGUAGES 的插件不一致");
        for (plugin, _) in LANGUAGES {
            assert!(config[config_key(plugin)].is_object(), "dprint.json 缺少 {}（{}）設定", config_key(plugin), plugin);
        }
    }

    /// 端到端煙霧測試：先執行 `make formatters`（或 scripts/fetch_formatter.sh）
    /// 下載固定版本的執行檔與插件，再以
    /// `cargo test dprint_formats_c_via_bundled_helper -- --ignored` 執行。
    #[test]
    #[ignore = "需要先下載 dprint 與插件（make formatters）"]
    fn dprint_formats_c_via_bundled_helper() {
        let formatted = format_text("int main(){int x=1;return x;}", "main.cpp").expect("格式化成功");
        assert!(formatted.contains("int main()"), "輸出：{formatted}");
        // 全域 indentWidth = 4（clang-format 0.2.1 外掛不接受區段內選項）
        assert!(formatted.contains("    int x = 1;"), "應採用 4 空格縮排：{formatted}");
        // 不支援的格式要拒絕，而不是原樣返回
        let err = format_text("fn main() {}", "main.rs").unwrap_err();
        assert!(err.contains("沒有支援"), "錯誤訊息：{err}");
    }
}
