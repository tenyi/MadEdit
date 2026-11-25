//! 腳本運行器：依副檔名選擇直譯器（.nut → nut、.py → python、.lua → lua、
//! .ps1/.bat → PowerShell/cmd；`*.ztrtoml` → ztr --ztrtoml），
//! 後台執行目前檔案（或選中的片段），輸出串流到底部「運行輸出」面板。
//!
//! Windows 主控台程式常以 GBK/ANSI 輸出，逐行解碼時先試嚴格 UTF-8，失敗則以 GBK 解碼。

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use eframe::egui::{self, Align2, RichText};

use super::{form, menus, App};
use crate::app::fonts::EDITOR_FAMILY;
use crate::app::i18n;
use crate::core::encoding;

/// Runner 訊息：一行輸出，或子程序結束
enum Msg {
    Output { text: String, starts_line: bool, ends_line: bool },
    InputFailed(String),
    Done(Result<i32, String>),
}

// Keep both memory use and delivery latency bounded for output without newlines.
// A decoder belongs to one logical line so a chunk boundary may split a character.
fn forward_output(pipe: impl Read, tx: SyncSender<Msg>) {
    const CHUNK: u64 = 16 * 1024;
    let mut reader = BufReader::new(pipe);
    let mut bytes = Vec::with_capacity(CHUNK as usize);
    let mut decoder = None;
    loop {
        bytes.clear();
        let Ok(n) = Read::by_ref(&mut reader).take(CHUNK).read_until(b'\n', &mut bytes) else { break };
        if n == 0 && decoder.is_none() {
            break;
        }
        let ends_line = bytes.ends_with(b"\n") || n < CHUNK as usize;
        let starts_line = decoder.is_none();
        let decoder_ref = decoder.get_or_insert_with(|| {
            let encoding = match std::str::from_utf8(&bytes) {
                Err(err) if err.error_len().is_some() => encoding_rs::GBK,
                _ => encoding_rs::UTF_8,
            };
            encoding.new_decoder_without_bom_handling()
        });
        let mut text = String::with_capacity(decoder_ref.max_utf8_buffer_length(bytes.len()).unwrap());
        let (_, consumed, _) = decoder_ref.decode_to_string(&bytes, &mut text, ends_line);
        debug_assert_eq!(consumed, n);
        if ends_line {
            if text.ends_with('\n') {
                text.pop();
                if text.ends_with('\r') {
                    text.pop();
                }
            }
            decoder = None;
        }
        if !(starts_line && ends_line && text.is_empty())
            && tx.send(Msg::Output { text, starts_line, ends_line }).is_err()
        {
            break;
        }
    }
}

/// 面板保留的輸出行數上限。面板每次內容變動都要重新排版整段文字，行數無限成長會讓
/// 整個 UI 越來越慢（實測 2 萬行約 45ms/幀、10 萬行約 230ms/幀），因此只保留尾段。
pub(crate) const MAX_OUTPUT_LINES: usize = 5_000;

/// 面板保留的輸出位元組上限（UTF-8 約等於排版成本；5 千行 × 40 字元 ≈ 200KB）。
const MAX_OUTPUT_CHARS: usize = 200_000;

/// 沒有腳本路徑（如未存檔的選取片段）時，退回寫到 MadEdit.exe 同目錄的 result.log
const LOG_FILE_NAME: &str = "result.log";

/// 一次運行的狀態與輸出
pub struct ScriptRunner {
    pub show: bool,
    pub running: bool,
    pub lines: Vec<String>,
    /// 輸出全文（= lines 以換行相接）；供可選取的輸出文字框顯示與複製
    pub text: String,
    /// 因超過上限而被省略的行數（面板顯示提示用）
    pub trimmed_lines: usize,
    /// 右鍵選單勾選「輸出到檔案」：所有輸出同時寫入記錄檔（無大小限制，
    /// 面板截斷不影響檔案內容）
    pub log_enabled: bool,
    /// 這次 F5 運行的腳本檔案；記錄檔預設放「腳本所在目錄／腳本檔名.result」
    /// （`a.nut` → `a.nut.result`），每個腳本的日誌跟著腳本走、互不覆蓋
    pub(crate) script_path: Option<PathBuf>,
    /// 記錄檔位置；Some 時完全覆寫預設規則（測試可覆寫到暫存目錄）
    pub(crate) log_path: Option<PathBuf>,
    /// 已開啟的記錄檔；停用時關閉
    log_file: Option<std::fs::File>,
    /// 運行結束的結果：退出碼或啟動失敗訊息
    pub exit: Option<Result<i32, String>>,
    pub started: Option<Instant>,
    /// 結束時凍結的耗時（秒）；運行中為 None，由 started 即時計算
    pub elapsed: Option<f32>,
    rx: Option<Receiver<Msg>>,
    child: Option<Arc<Mutex<Child>>>,
    /// 子程序 PID：停止時按 PID 殺整棵程序樹（cmd → py → GUI 不留孤兒）
    pid: Option<u32>,
    /// 等待子程序結束的執行緒（單位型 JoinHandle）
    worker: Option<JoinHandle<()>>,
    /// A bounded queue keeps pipe writes off the UI thread.
    stdin: Option<SyncSender<String>>,
    /// 輸入行的內容（運行中顯示在面板底部的提示符輸入框）
    pub(crate) input: String,
    /// 程序剛啟動：面板下一次畫出輸入框時向它要一次焦點（只搶一次，不打斷編輯）
    pub(crate) focus_input: bool,
    /// 跟隨捲動游標：已經「跟到底」過的輸出行數；有新行到達時面板再捲一次到底
    pub(crate) follow_seen: usize,
    /// 選中片段的臨時檔案，運行結束後刪除
    temp_file: Option<tempfile::TempPath>,
    /// 使用者按下停止
    pub stopped: bool,
    /// 最近一次有效的選取範圍（字元索引），供右鍵「複製」使用；
    /// 文字框失焦時 egui 會收合選取，因此另行記住
    sel: Option<(usize, usize)>,
}

impl Default for ScriptRunner {
    fn default() -> Self {
        Self {
            show: false,
            running: false,
            lines: Vec::new(),
            text: String::new(),
            trimmed_lines: 0,
            log_enabled: false,
            script_path: None,
            log_path: None,
            log_file: None,
            exit: None,
            started: None,
            elapsed: None,
            rx: None,
            child: None,
            stdin: None,
            input: String::new(),
            focus_input: false,
            follow_seen: 0,
            pid: None,
            worker: None,
            temp_file: None,
            stopped: false,
            sel: None,
        }
    }
}

impl ScriptRunner {
    /// 新增一行輸出（同步維護可選取文字框用的全文）
    pub(crate) fn push_line(&mut self, line: String) {
        self.push_output(line, true, true);
    }

    fn push_output(&mut self, text: String, starts_line: bool, ends_line: bool) {
        if let Some(log) = &mut self.log_file {
            let _ = log.write_all(text.as_bytes());
            if ends_line {
                let _ = log.write_all(b"\n");
            }
        }
        if starts_line || self.lines.is_empty() {
            if !self.lines.is_empty() {
                self.text.push('\n');
            }
            self.lines.push(String::new());
        }
        self.text.push_str(&text);
        self.lines.last_mut().unwrap().push_str(&text);
        self.trim_output();
    }

    /// 只保留最後一段輸出。面板每次內容變動都要重新排版整段文字，輸出無限成長會讓整個 UI
    /// 越來越慢，所以超過 [`MAX_OUTPUT_LINES`]／[`MAX_OUTPUT_CHARS`] 時從最前面丟掉，
    /// 並記下丟了幾行（面板顯示提示，免得讓人以為程式只印了這些）。
    fn trim_output(&mut self) {
        if self.lines.len() <= MAX_OUTPUT_LINES && self.text.len() <= MAX_OUTPUT_CHARS {
            return;
        }
        // 一次砍到上限的九成，避免之後每一行都要重建一次字串
        let keep_chars = MAX_OUTPUT_CHARS * 9 / 10;
        let mut drop = 0;
        let mut bytes = self.text.len();
        let mut count = self.lines.len();
        while drop + 1 < self.lines.len() && (count > MAX_OUTPUT_LINES || bytes > keep_chars) {
            bytes -= self.lines[drop].len() + 1; // + 換行
            count -= 1;
            drop += 1;
        }
        if drop > 0 {
            self.lines.drain(..drop);
            self.trimmed_lines += drop;
            // 選取是字元索引，內容一變就失效
            self.sel = None;
            self.text = self.lines.join("\n");
        }
        // 單行就可能超過預算（例如一次印出幾 MB 的 JSON）：只保留結尾，
        // 否則排版成本無法收斂，面板會整個卡住
        if self.text.len() > MAX_OUTPUT_CHARS
            && let Some(last) = self.lines.last_mut()
        {
            let mut cut = last.len() - keep_chars;
            while cut < last.len() && !last.is_char_boundary(cut) {
                cut += 1;
            }
            *last = format!("…{}", &last[cut..]);
            self.trimmed_lines += 1;
            self.sel = None;
            self.text = self.lines.join("\n");
        }
    }

    /// 清空輸出（面板按鈕與右鍵選單共用）。記錄檔不受影響：只有 F5 開始新運行才從頭寫。
    fn clear_output(&mut self) {
        self.lines.clear();
        self.text.clear();
        self.trimmed_lines = 0;
        self.exit = None;
        self.stopped = false;
        self.sel = None;
    }

    /// 停止目前子程序：按 PID 殺掉整棵程序樹（cmd 與它啟動的子程序一起結束）。
    ///
    /// 不鎖 child 互斥鎖（等待執行緒輪詢時只短暫持鎖，但為避免任何 UI 卡頓，
    /// Windows 直接用 taskkill 在背景執行緒殺樹）；程序結束後由 poll 收尾。
    pub fn stop(&mut self) {
        self.stopped = true;
        self.kill_tree();
    }

    /// 記錄檔的完整路徑：優先使用者覆寫（log_path），否則「腳本所在目錄／腳本檔名.result」
    /// （`a.nut` → `a.nut.result`、`a.py` → `a.py.result`；直接在腳本名後追加 .result，
    /// 檔名排序時與腳本相鄰、綁定關係一目了然，主名與副檔名組合唯一，互不覆蓋）；
    /// 沒有腳本路徑時退回 MadEdit.exe 同目錄的 result.log
    pub(crate) fn resolved_log_path(&self) -> Option<PathBuf> {
        if let Some(p) = &self.log_path {
            return Some(p.clone());
        }
        if let Some(script) = &self.script_path
            && let (Some(dir), Some(name)) = (script.parent(), script.file_name())
        {
            let mut log = name.to_string_lossy().into_owned();
            log.push_str(".result");
            return Some(dir.join(log));
        }
        std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.join(LOG_FILE_NAME)))
    }

    /// 勾選／取消「輸出到檔案」。開啟時**以清空方式開檔**，並把面板目前的輸出整段寫入
    /// （兩邊同步：檔案內容＝面板顯示的內容），之後新輸出接在後面；失敗時回傳錯誤訊息並保持停用。
    pub(crate) fn set_log_enabled(&mut self, on: bool) -> Result<(), String> {
        self.set_log_with_mode(on, true)
    }

    /// 啟動時沿用上次的勾選：以**續寫**模式開檔（清空只在 F5 開始新運行或面板勾選同步時發生）。
    pub(crate) fn set_log_enabled_append(&mut self, on: bool) -> Result<(), String> {
        self.set_log_with_mode(on, false)
    }

    fn set_log_with_mode(&mut self, on: bool, sync_with_panel: bool) -> Result<(), String> {
        if !on {
            self.log_enabled = false;
            self.log_file = None; // 關閉檔案
            return Ok(());
        }
        let Some(path) = self.resolved_log_path() else {
            return Err("cannot locate the executable directory".into());
        };
        let open = || {
            if sync_with_panel {
                std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&path)
            } else {
                std::fs::OpenOptions::new().create(true).append(true).open(&path)
            }
        };
        match open() {
            Ok(mut f) => {
                if sync_with_panel {
                    // 同步：面板目前顯示的內容整段寫入（含每行結尾換行），之後的新輸出接在後面
                    for line in &self.lines {
                        if f.write_all(line.as_bytes()).is_err() || f.write_all(b"\n").is_err() {
                            return Err(path.display().to_string());
                        }
                    }
                }
                self.log_file = Some(f);
                self.log_enabled = true;
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// F5 開始新運行：記錄檔從頭寫。失敗時回傳錯誤，由呼叫端通知使用者並停用。
    pub(crate) fn reopen_log_truncated(&mut self) -> std::io::Result<()> {
        if !self.log_enabled {
            return Ok(());
        }
        let Some(path) = self.resolved_log_path() else { return Ok(()) };
        self.log_file = Some(std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(path)?);
        Ok(())
    }

    /// 停用記錄並關閉檔案（開檔失敗時由呼叫端呼叫）
    pub(crate) fn disable_log(&mut self) {
        self.log_enabled = false;
        self.log_file = None;
    }

    /// 殺掉子程序樹；不在 UI 執行緒等待任何鎖或程序
    fn kill_tree(&mut self) {
        let Some(pid) = self.pid else { return };
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let pid = pid.to_string();
            let _ = Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid])
                .creation_flags(CREATE_NO_WINDOW)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
        #[cfg(not(windows))]
        if let Some(child) = &self.child {
            // 等待執行緒只短暫持鎖輪詢，這裡可安全取得並 kill
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
            }
        }
        let _ = pid;
    }

    /// 啟動子程序；stdout 與 stderr 各由一條執行緒逐行轉發
    pub fn start_process(&mut self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<(), String> {
        if self.running {
            return Err("script already running".into());
        }
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 輸出已重定向到面板：隱藏 cmd/powershell 等子程序自己彈出的主控台黑窗
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn().map_err(|e| format!("{program}: {e}"))?;
        let pid = child.id();
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut stdin = child.stdin.take().expect("stdin piped");
        let (tx, rx) = sync_channel::<Msg>(32);
        let input_errors = tx.clone();
        let (input_tx, input_rx) = sync_channel::<String>(4);
        std::thread::spawn(move || {
            for line in input_rx {
                if let Err(err) = stdin.write_all(line.as_bytes()).and_then(|()| stdin.write_all(b"\r\n")) {
                    let _ = input_errors.send(Msg::InputFailed(err.to_string()));
                    break;
                }
            }
        });
        let child = Arc::new(Mutex::new(child));
        let mut pipe_workers = Vec::new();

        // stdout 與 stderr 型別不同：分別包裝成統一的 Read 再交給同一條讀取邏輯
        let readers: Vec<Box<dyn Read + Send>> = vec![Box::new(stdout), Box::new(stderr)];
        for pipe in readers {
            let tx = tx.clone();
            pipe_workers.push(std::thread::spawn(move || forward_output(pipe, tx)));
        }
        let temp_file = self.temp_file.take();
        self.worker = Some({
            let child = Arc::clone(&child);
            std::thread::spawn(move || {
                // 短鎖輪詢 try_wait：絕不持鎖阻塞，否則 UI 執行緒 stop() 會死鎖
                let code = loop {
                    let polled = child.lock().expect("runner process lock").try_wait();
                    match polled {
                        Ok(Some(status)) => break Ok(status.code().unwrap_or(-1)),
                        Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
                        Err(err) => break Err(err.to_string()),
                    }
                };
                for reader in pipe_workers {
                    let _ = reader.join();
                }
                drop(temp_file);
                let _ = tx.send(Msg::Done(code));
            })
        });
        self.child = Some(child);
        self.pid = Some(pid);
        self.stdin = Some(input_tx);
        self.input.clear();
        self.focus_input = true;
        self.rx = Some(rx);
        self.running = true;
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.exit = None;
        self.stopped = false;
        Ok(())
    }

    /// 收取子程序輸出與結束事件；背景執行緒活著時安排重繪
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut input_error = None;
        // 先取出接收端，才能在同一輪迴圈裡同時收訊息與改動自身
        let rx = self.rx.take()?;
        let mut keep_rx = true;
        for _ in 0..64 {
            match rx.try_recv() {
                Ok(Msg::Output { text, starts_line, ends_line }) => self.push_output(text, starts_line, ends_line),
                Ok(Msg::InputFailed(err)) => input_error = Some(err),
                Ok(Msg::Done(result)) => {
                    self.running = false;
                    // 凍結耗時：結束後面板不再繼續計時
                    self.elapsed = self.started.map(|t| t.elapsed().as_secs_f32());
                    self.exit = Some(match result {
                        Ok(code) => Ok(code),
                        Err(e) => {
                            self.push_line(format!("{e}"));
                            Err(e)
                        }
                    });

                    if let Some(worker) = self.worker.take() {
                        let _ = worker.join();
                    }
                    self.child = None;
                    self.pid = None;
                    // 關閉 stdin（子程序讀到 EOF），輸入行隨面板一起消失
                    self.stdin = None;
                    self.input.clear();
                    ctx.request_repaint();
                    // 結束後不再放回接收端
                    keep_rx = false;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.running = false;
                    keep_rx = false;
                    break;
                }
            }
        }
        if keep_rx {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            self.rx = Some(rx);
        }
        input_error
    }

    /// 把一行輸入寫進子程序 stdin（面板輸入行按 Enter 送出）。
    /// 同時以「> 行」回顯到輸出與記錄檔，方便對照腳本停在什麼提示上。
    /// 程序已結束或管道寫入失敗時回傳錯誤，由呼叫端提示使用者。
    pub fn send_line(&mut self, line: &str) -> Result<(), String> {
        self.stdin.as_ref().ok_or("process exited")?
            .try_send(line.to_owned()).map_err(|err| err.to_string())?;
        self.push_line(format!("> {line}"));
        Ok(())
    }
}

impl Drop for ScriptRunner {
    /// 結束程式時不要留下執行中的子程序（連同它啟動的子程序一起結束）
    fn drop(&mut self) {
        if self.pid.is_some() {
            self.kill_tree();
        }
    }
}

/// 把選取片段寫入暫存檔（保留原副檔名，直譯器才知道語言）。
/// 內容以呼叫端提供的位元組寫入，讓暫存檔沿用原檔編碼（例如 GBK）。
pub fn write_selection_temp(name: &str, bytes: &[u8]) -> std::io::Result<tempfile::TempPath> {
    let suffix = Path::new(name).extension().and_then(|ext| ext.to_str()).unwrap_or_default();
    let mut file = tempfile::Builder::new().prefix("madedit_run_").suffix(&format!(".{suffix}")).tempfile()?;
    file.write_all(bytes)?;
    Ok(file.into_temp_path())
}

/// 依副檔名選擇直譯器。`.nut` 優先使用 MadEdit.exe 同目錄的 nut.exe（Squirrel 主控台），
/// 找不到才依賴 PATH；其餘 `.py`、`.lua` 依賴系統 PATH。回傳（程式、前置參數）。
pub fn interpreter_for(ext: &str) -> Option<(String, Vec<String>)> {
    match ext {
        ".nut" => {
            let exe = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("nut.exe")))
                .filter(|p| p.exists())
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "nut".into());
            Some((exe, Vec::new()))
        }
        ".py" => Some(("python".into(), vec!["-u".into()])),
        ".lua" => Some(("lua".into(), Vec::new())),
        // Windows PowerShell：-NoProfile 加速啟動、Bypass 允許本地腳本、File 指定腳本路徑
        ".ps1" => Some(("powershell".into(), vec!["-NoLogo".into(), "-NoProfile".into(), "-ExecutionPolicy".into(), "Bypass".into(), "-File".into()])),
        // Windows 批處理：交給 cmd /c 執行（工作目錄由呼叫端設為 .bat 所在資料夾）
        ".bat" | ".cmd" => Some(("cmd".into(), vec!["/c".into()])),
        _ => None,
    }
}

impl App {
    /// 工具 → 運行腳本（F5）：有選取時只運行選中片段，否則運行整個檔案
    pub fn run_script(&mut self) {
        if self.runner.running {
            self.runner.show = true;
            return;
        }
        // `*.ztrtoml`：ztr 壓縮配置檔，直接在所在目錄執行 `ztr --ztrtoml <檔名>`。
        if self.run_ztr_config() {
            return;
        }
        let ext = self
            .doc()
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_ascii_lowercase()))
            .unwrap_or_default();
        let Some((program, mut args)) = interpreter_for(&ext) else {
            let hint = self.t("run.unknown_ext", "無法識別的腳本類型：{q}").replace("{q}", &ext);
            self.set_status(hint);
            return;
        };
        let (selection, encoding, has_bom) = {
            let doc = self.doc();
            let text = doc.editor.selected_text();
            let selection = (!text.is_empty()).then_some(text);
            (selection, doc.encoding, doc.has_bom)
        };
        let file: PathBuf = match selection {
            // 選中片段：寫入同副檔名的臨時檔案，只運行這一段。
            // 內容沿用原檔編碼（例如 GBK），否則直譯器會把 UTF-8 位元組當成 ANSI 而亂碼。
            Some(text) => {
                let encoded = match encoding::encode_text(&text, encoding, has_bom) {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        let hint = self
                            .t("run.encode_fail", "選取內容無法以目前編碼（{q}）儲存")
                            .replace("{q}", encoding.name());
                        self.set_status(hint);
                        return;
                    }
                };
                let tmp = match write_selection_temp(&self.doc().display_name(), &encoded) {
                    Ok(tmp) => tmp,
                    Err(e) => {
                        let hint = self.t("run.temp_fail", "無法寫入臨時檔案：{q}").replace("{q}", &e.to_string());
                        self.set_status(hint);
                        return;
                    }
                };
                let path = tmp.to_path_buf();
                self.runner.temp_file = Some(tmp);
                path
            }
            None => {
                let Some(path) = self.doc().path.clone() else {
                    self.set_status(self.t("run.no_save", "請先儲存檔案再運行"));
                    return;
                };
                if self.doc().is_modified() {
                    let i = self.active;
                    if !self.save_tab(i, false) || self.doc().is_modified() {
                        return;
                    }
                }
                path
            }
        };
        // cmd 等外部程式不認得 `\\?\` verbatim 路徑（cd /d "%~dp0" 會失敗），先清掉前綴
        let file = crate::core::favorites::clean_path(file);
        // 記錄檔跟著腳本走：所在目錄／腳本檔名.result（選取片段未有檔案時用臨時檔位置）
        self.runner.script_path = self.doc().path.clone().or_else(|| Some(file.clone()));
        args.push(file.display().to_string());
        let cwd = self.doc().path.clone().map(crate::core::favorites::clean_path)
            .and_then(|source| source.parent().map(Path::to_owned));
        self.launch_runner(program, args, cwd.as_deref());
    }

    /// `*.ztrtoml` 的 F5：在該檔所在目錄執行 `ztr --ztrtoml <檔名>`（無子命令＝壓縮當前目錄）。
    /// 回傳 true 表示文件是 ztr 配置檔（無論是否成功啟動）；false 交回一般腳本流程。
    fn run_ztr_config(&mut self) -> bool {
        let Some(path) = self.doc().path.clone() else {
            return false;
        };
        let is_ztr = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(crate::core::syntax::is_ztr_config_name);
        if !is_ztr {
            return false;
        }
        if self.doc().is_modified() {
            let i = self.active;
            if !self.save_tab(i, false) || self.doc().is_modified() {
                // 儲存失敗或被取消：是 ztr 配置檔這一點不變，但不再啟動
                return true;
            }
        }
        // cmd 等外部程式不認得 `\\?\` verbatim 路徑，先清掉前綴；以檔名傳參，工作目錄即其所在目錄
        let file = crate::core::favorites::clean_path(path);
        let Some(dir) = file.parent().map(|p| p.to_path_buf()) else {
            return true;
        };
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // 優先使用 MadEdit.exe 同目錄的 ztr.exe（可與 MadEdit 捆綁發佈），找不到才依賴 PATH
        let program = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("ztr.exe")))
            .filter(|p| p.exists())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "ztr".into());
        let args = vec!["--ztrtoml".to_string(), name];
        self.runner.script_path = Some(file.clone());
        self.launch_runner(program, args, Some(&dir));
        true
    }

    /// 啟動子程序前清空輸出面板與記錄檔；啟動失敗立即清理臨時檔並顯示錯誤。
    fn launch_runner(&mut self, program: String, args: Vec<String>, cwd: Option<&std::path::Path>) {
        self.runner.show = true;
        self.runner.lines.clear();
        self.runner.text.clear();
        self.runner.trimmed_lines = 0;
        self.runner.follow_seen = 0;
        // 開啟「輸出到檔案」時，新的 F5 運行從頭寫記錄檔（腳本同名.result）；開檔失敗就停用並告知
        if let Err(e) = self.runner.reopen_log_truncated() {
            self.runner.disable_log();
            let hint = format!("{} {e}", self.t("run.log_fail", "無法開啟記錄檔："));
            self.set_status(hint);
        }
        if let Err(e) = self.runner.start_process(&program, &args, cwd) {
            // 啟動失敗時不會有 Done 事件，立即清掉暫存檔
            self.runner.temp_file = None;
            self.runner.exit = Some(Err(e.clone()));
            self.runner.push_line(e);
        }
    }

    /// 底部「運行輸出」面板
    pub fn script_output_panel(&mut self, ui: &mut egui::Ui) {
        let lang = self.ui_lang();
        let title = self.t("run.title", "運行輸出");
        let running = self.runner.running;
        let exit = self.runner.exit.clone();
        let elapsed = if running {
            self.runner.started.map(|t| t.elapsed().as_secs_f32())
        } else {
            self.runner.elapsed
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong());
            if running {
                ui.add(egui::Spinner::new());
                ui.label(RichText::new(self.t("run.running", "運行中…")).weak());
            } else if let Some(Ok(code)) = &exit {
                let secs = elapsed.map(|s| format!("{s:.3}")).unwrap_or_default();
                let text = self
                    .t("run.done", "退出碼: {n} · 耗時: {q}s")
                    .replace("{n}", &code.to_string())
                    .replace("{q}", &secs);
                let color = if *code == 0 { ui.visuals().weak_text_color() } else { ui.visuals().warn_fg_color };
                ui.label(RichText::new(text).color(color));
            } else if let Some(Err(_)) = &exit {
                ui.label(RichText::new(self.t("run.failed", "啟動失敗")).color(ui.visuals().error_fg_color));
            }
            if self.runner.stopped {
                ui.label(RichText::new(self.t("run.stopped", "已停止")).color(ui.visuals().warn_fg_color));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if form::icon_button(ui, egui::Button::new("×").frame(false), self.t("btn.close", "關閉"), self.t("btn.close", "關閉")).clicked() {
                    self.runner.show = false;
                }
                if ui.button(self.t("opt.clear", "清除")).clicked() {
                    self.runner.clear_output();
                }
                if running && ui.button(self.t("run.stop", "停止")).clicked() {
                    self.runner.stop();
                }
            });
        });
        ui.separator();
        let running = self.runner.running;
        let has_output = !self.runner.text.is_empty();
        // 輸入行錨定在面板內部矩形的最底端，作為**懸浮層**畫在輸出區之上（帶面板底色，
        // 蓋住捲到最下面的輸出，看起來就像終端把提示符釘在最後一行）。這個面板的高度＝
        // 上一幀的內容高度，所以內容必須**恰好填滿**面板分配的高度：輸出區佔滿剩餘空間
        //（與未加輸入行前完全相同），輸入行不進內容流、不回報額外高度——內容高度恆等於
        // 分配高度，面板在任一縮放係數下都不漂移。輸入行若也走正常排版流，無論預留多少
        // 都會跟面板高度互相回饋（實測要嘛一幀長一點、要嘛一幀縮一點）。
        let full = ui.max_rect();
        let row_h = self.settings.font_size + ui.spacing().item_spacing.y;
        let row_rect = egui::Rect::from_min_max(
            egui::pos2(full.left(), full.bottom() - row_h),
            egui::pos2(full.right(), full.bottom()),
        );
        // 輸出被截斷時要講清楚，免得讓人以為程式只印了這些
        if self.runner.trimmed_lines > 0 {
            let hint = self.tf("run.trimmed", "（輸出過長，已省略較早的 {n} 行）", Some(self.runner.trimmed_lines), None);
            ui.label(RichText::new(hint).weak());
        }
        // 輸出與編輯區使用相同字型與字級（含等寬/比例設定由編輯區字型決定）
        let output_font = egui::FontId::new(self.settings.font_size, egui::FontFamily::Name(EDITOR_FAMILY.into()));
        let empty_center = ui.available_rect_before_wrap().center();
        // 可選取的唯讀文字框：傳入 &str 讓文字框只可選取、複製而不能編輯；
        // 外層捲動區保留長輸出的垂直捲動，右鍵選單掛在整個面板上（含空白處）
        let mut menu = OutputMenu::default();
        let mut state = None;
        let mut text_id = None;
        let mut selected = None;
        // 運行中有新輸出到達時跟隨捲動到最後一行（以行數計變化，避免每幀搶捲動）
        let line_count = self.runner.lines.len();
        let stick = running && line_count != self.runner.follow_seen;
        let body = ui
            .scope(|ui| {
                egui::ScrollArea::vertical().id_salt("script-output").auto_shrink([false, false]).show(ui, |ui| {
                    let mut text = self.runner.text.as_str();
                    let output = egui::TextEdit::multiline(&mut text)
                        .id_salt("script-output-text")
                        .frame(egui::Frame::NONE)
                        .font(output_font.clone())
                        .desired_width(ui.available_width())
                        .desired_rows(1)
                        .show(ui);
                    text_id = Some(output.response.id);
                    // 運行中在捲動內容末尾墊出輸入行高度的空檔：捲到底時最後一行輸出
                    // 剛好停在輸入行上方，不會被懸浮的輸入行蓋住（空檔在捲動內容裡，
                    // 不影響「內容＝面板高度」的恆等，面板高度不受影響）
                    if running {
                        ui.add_space(row_h);
                    }
                    if stick {
                        ui.scroll_to_rect(ui.min_rect(), Some(egui::Align::BOTTOM));
                    }
                    if let Some(range) = output.cursor_range {
                        let (a, b) = (range.primary.index.min(range.secondary.index).0, range.primary.index.max(range.secondary.index).0);
                        if a < b {
                            selected = Some((a, b));
                        }
                    }
                    state = Some(output.state);
                });
            })
            .response;
        if stick {
            self.runner.follow_seen = line_count;
        }
        // 右鍵選單：在「放開」時開啟，按住多久都開得出來。egui 的 click 只認 0.8 秒內的短按，
        // 按住不放會被當成長按而不算 click，選單就永遠不會開；位置固定在放開時的游標處。
        let right_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Secondary)) && body.container_contains_pointer();
        let open = if right_released {
            Some(egui::SetOpenCommand::Bool(true))
        } else if body.container_clicked() {
            // 左鍵點面板時關掉選單（與 egui 內建的 container_context_menu 一致）
            Some(egui::SetOpenCommand::Bool(false))
        } else {
            None
        };
        egui::Popup::menu(&body).open_memory(open).at_pointer_fixed().show(|ui| output_menu(ui, lang, self.runner.log_enabled, &mut menu));
        if selected.is_some() {
            self.runner.sel = selected;
        }
        // 「輸出到檔案」是即時生效的勾選項：勾上以面板目前內容同步記錄檔，取消就關檔；
        // 勾選狀態存進設定，重開程式沿用（啟動時以續寫模式開檔，不清空）
        if menu.toggle_log {
            let on = !self.runner.log_enabled;
            match self.runner.set_log_enabled(on) {
                Ok(()) => {
                    self.settings.output_log_to_file = on;
                    self.save_settings();
                    if on
                        && let Some(path) = self.runner.resolved_log_path()
                    {
                        let hint = self.tf("run.log_on", "輸出同時寫入 {q}", None, Some(&path.display().to_string()));
                        self.set_status(hint);
                    }
                }
                Err(e) => {
                    let hint = format!("{} {e}", self.t("run.log_fail", "無法開啟記錄檔："));
                    self.set_status(hint);
                }
            }
        }
        if menu.copy {
            let text = selected_or_all(&self.runner.text, self.runner.sel);
            self.copy_to_system(&text);
            self.set_status(self.t("run.copied", "已複製輸出到剪貼簿"));
        }
        if menu.clear {
            self.runner.clear_output();
        }
        // 右鍵不清掉選取（Windows 習慣，與編輯區一致）：egui 把次要鍵「按下」當成重新定位
        // 游標，會把選取收合成一個插入點；所以在按下那一幀就把記住的範圍寫回去，
        // 不能等放開才做，否則按住期間選取會消失、放開又回來，看起來像閃爍。
        let right_pressed = ui.input(|i| i.pointer.secondary_pressed()) && body.container_contains_pointer();
        let keep = if menu.select_all {
            Some((0, self.runner.text.chars().count()))
        } else if right_pressed {
            self.runner.sel
        } else {
            None
        };
        if let Some((a, b)) = keep {
            let len = self.runner.text.chars().count();
            let (a, b) = (a.min(b).min(len), a.max(b).min(len));
            if a < b {
                if let (Some(mut state), Some(id)) = (state, text_id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b))));
                    state.store(ui.ctx(), id);
                    ui.memory_mut(|mem| mem.request_focus(id));
                }
            }
        }
        // 運行中在面板最底部顯示命令列提示符式輸入行：懸浮在面板底色上（蓋住捲到下面的輸出），
        // 無邊框，看起來像終端的最後一行。回車把一行寫進子程序 stdin（set /p、input() 等
        // 行式互動可用；pause、choice 直接讀主控台按鍵，管道救不了，只能停止或改用真終端）。
        if running {
            ui.painter().rect_filled(row_rect, 0.0, ui.visuals().panel_fill);
            let mut row = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("run-input-row")
                    .max_rect(row_rect)
                    .layout(egui::Layout::top_down(egui::Align::LEFT)),
            );
            row.horizontal(|ui| {
                let prompt = RichText::new("> ").font(output_font.clone()).color(ui.visuals().text_color());
                ui.label(prompt);
                let hint = self.t("run.input_hint", "輸入並按 Enter 傳送到程序…");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.runner.input)
                        .id_salt("run-input")
                        .frame(egui::Frame::NONE)
                        .hint_text(hint)
                        .font(output_font.clone())
                        .desired_width(ui.available_width()),
                );
                // 只在程序啟動後的第一幀搶一次焦點（游標閃爍提示可輸入），
                // 之後不干擾編輯區；送出後繼續留在輸入框方便連續輸入
                if self.runner.focus_input {
                    self.runner.focus_input = false;
                    resp.request_focus();
                }
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                // egui 的單行 TextEdit 在按 Enter 時會**先交出焦點**，所以不能用 has_focus 判斷，
                // 必須用「失去焦點的那一幀且按著 Enter」：點別處失焦不送出，按 Enter 送出
                if resp.lost_focus() && enter {
                    let line = std::mem::take(&mut self.runner.input);
                    if let Err(e) = self.runner.send_line(&line) {
                        let hint = format!("{} {e}", self.t("run.input_fail", "無法傳送到程序（可能已結束）："));
                        self.runner.input = line;
                        self.set_status(hint);
                    }
                    resp.request_focus();
                }
            });
        }
        if !has_output && !running {
            ui.painter().text(empty_center, Align2::CENTER_CENTER, self.t("run.empty", "（尚無輸出）"), output_font, ui.visuals().weak_text_color());
        }
        if running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

/// 輸出面板右鍵選單的動作（全選／複製／清空／輸出到檔案）
#[derive(Default)]
struct OutputMenu {
    select_all: bool,
    copy: bool,
    clear: bool,
    toggle_log: bool,
}

/// 輸出面板右鍵選單；空白處與文字框共用同一份選單。
/// `log_enabled` 只用來畫「輸出到檔案」的勾選狀態，切換在選單關閉後統一處理。
fn output_menu(ui: &mut egui::Ui, lang: i18n::Lang, log_enabled: bool, menu: &mut OutputMenu) {
    use crate::app::i18n::translate;
    ui.set_min_width(menus::SUBMENU_MIN_WIDTH);
    if ui.button(menus::check_label(false, translate(lang, "run.select_all", "全選"))).clicked() {
        menu.select_all = true;
        ui.close();
    }
    if ui.button(menus::check_label(false, translate(lang, "run.copy", "複製"))).clicked() {
        menu.copy = true;
        ui.close();
    }
    if ui.button(menus::check_label(false, translate(lang, "run.clear", "清空"))).clicked() {
        menu.clear = true;
        ui.close();
    }
    // 複選型選單項：點了直接切換狀態，選單保持開著（勾選狀態下幀就會更新）
    ui.separator();
    if ui.button(menus::check_label(log_enabled, translate(lang, "run.log_to_file", "輸出到檔案（腳本同名.result）"))).clicked() {
        menu.toggle_log = true;
        ui.close();
    }
}

/// 取得要複製的文字：有選取範圍時取選中的字元區間，否則全文
fn selected_or_all(text: &str, sel: Option<(usize, usize)>) -> String {
    let Some((a, b)) = sel else { return text.to_string() };
    let len = text.chars().count();
    let (a, b) = (a.min(b).min(len), a.max(b).min(len));
    if a >= b {
        return text.to_string();
    }
    text.chars().skip(a).take(b - a).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpreter_mapping_by_extension() {
        let (nut, nut_args) = interpreter_for(".nut").expect("nut");
        // 優先 MadEdit.exe 同目錄的 nut.exe；測試環境沒有該檔時回退 PATH 名
        assert!(nut.to_lowercase().ends_with("nut.exe") || nut == "nut", "{nut}");
        assert!(nut_args.is_empty());
        let (py, py_args) = interpreter_for(".py").expect("py");
        assert_eq!(py, "python");
        assert_eq!(py_args, vec!["-u".to_string()]);
        assert_eq!(interpreter_for(".lua").expect("lua").0, "lua");
        // Windows 批處理交給 cmd /c，工作目錄已是 .bat 所在資料夾
        let (bat, bat_args) = interpreter_for(".bat").expect("bat");
        assert_eq!(bat, "cmd");
        assert_eq!(bat_args, vec!["/c".to_string()]);
        assert_eq!(interpreter_for(".cmd").expect("cmd").0, "cmd");
        let (ps1, ps1_args) = interpreter_for(".ps1").expect("ps1");
        assert_eq!(ps1, "powershell");
        assert_eq!(ps1_args, vec!["-NoLogo".to_string(), "-NoProfile".to_string(), "-ExecutionPolicy".to_string(), "Bypass".to_string(), "-File".to_string()]);
        assert!(interpreter_for(".txt").is_none());
        assert!(interpreter_for("").is_none());
    }

    #[test]
    fn test_ztr_config_name_detection() {
        use crate::core::syntax::is_ztr_config_name;
        assert!(is_ztr_config_name("pure-code.ztrtoml"));
        assert!(is_ztr_config_name("PURE-CODE.ZTRTOML"));
        assert!(is_ztr_config_name(".ztrtoml"));
        // 一般 toml 與舊式雙點名稱不是 ztr 配置檔
        assert!(!is_ztr_config_name("ztr.toml"));
        assert!(!is_ztr_config_name("config.toml"));
        assert!(!is_ztr_config_name("x.ztr.toml"));
        assert!(!is_ztr_config_name("x.ztrtoml.bak"));
    }

    #[test]
    fn test_selection_temp_file_keeps_content_and_extension() {
        let bytes = "print(\"hi\");\n".as_bytes();
        let tmp = write_selection_temp("madedit_test_selection.nut", bytes).expect("write temp");
        let other = write_selection_temp("madedit_test_selection.nut", b"second").unwrap();
        assert_ne!(tmp.to_path_buf(), other.to_path_buf());
        drop(other);
        assert_eq!(tmp.extension().and_then(|e| e.to_str()), Some("nut"));
        assert_eq!(std::fs::read(&tmp).unwrap(), bytes);
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_selection_temp_file_preserves_gbk_bytes() {
        // 選取片段沿用原檔編碼：GBK 檔案寫出的暫存檔必須仍是 GBK 位元組，
        // 否則 nut.exe 之類的 ANSI 直譯器會把中文當成亂碼。
        let text = "print(\"中文输出测试\");\n";
        let bytes = encoding::encode_text(text, encoding_rs::GBK, false).expect("gbk encodable");
        assert_ne!(bytes, text.as_bytes());
        let tmp = write_selection_temp("madedit_test_gbk.nut", &bytes).expect("write temp");
        let written = std::fs::read(&tmp).unwrap();
        assert_eq!(written, bytes);
        let (decoded, _, had_errors) = encoding_rs::GBK.decode(&written);
        assert!(!had_errors);
        assert_eq!(decoded, text);
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_selected_or_all_prefers_selection() {
        assert_eq!(selected_or_all("abc\ndef", None), "abc\ndef");
        assert_eq!(selected_or_all("abc\ndef", Some((0, 3))), "abc");
        assert_eq!(selected_or_all("abc\ndef", Some((4, 7))), "def");
        // 反向範圍（游標在選取起點左側）仍取同一段
        assert_eq!(selected_or_all("abc\ndef", Some((7, 4))), "def");
        // 空範圍與越界範圍都退回全文
        assert_eq!(selected_or_all("abc", Some((2, 2))), "abc");
        assert_eq!(selected_or_all("abc", Some((0, 99))), "abc");
        // 字元索引而非位元組索引
        assert_eq!(selected_or_all("中文abc", Some((1, 3))), "文a");
    }

    #[test]
    fn long_output_preserves_split_utf8_in_the_panel_and_log() {
        let text = format!("{}中文{}", "x".repeat(16383), "z".repeat(17000));
        let (tx, rx) = sync_channel(8);
        forward_output(text.as_bytes(), tx);
        let mut runner = ScriptRunner::default();
        let log = tempfile::NamedTempFile::new().unwrap();
        runner.log_path = Some(log.path().to_owned());
        runner.set_log_enabled(true).unwrap();
        for msg in rx {
            if let Msg::Output { text, starts_line, ends_line } = msg {
                runner.push_output(text, starts_line, ends_line);
            }
        }
        assert_eq!(runner.lines, [text.clone()]);
        assert_eq!(std::fs::read_to_string(log.path()).unwrap(), format!("{text}\n"));
    }

    /// stdin 互動：send_line 寫進子程序並以「> 行」回顯；程序結束後 stdin 關閉、輸入行清空
    #[test]
    fn test_send_line_feeds_child_stdin_and_echoes() {
        let mut runner = ScriptRunner::default();
        // Windows 用 findstr 過濾含 x 的行，其餘平台用 cat 原樣回傳
        #[cfg(windows)]
        let (program, args) = ("cmd".to_string(), vec!["/c".to_string(), "findstr".to_string(), "x".to_string()]);
        #[cfg(not(windows))]
        let (program, args) = ("cat".to_string(), Vec::new());
        runner.start_process(&program, &args, None).expect("spawn");
        assert!(runner.stdin.is_some(), "stdin is piped");
        runner.send_line("xline").expect("write stdin");
        // cat 讀到 EOF 才結束：關掉 stdin 端讓子程序收尾（cmd /c 會自己結束，不用關）
        runner.stdin = None;
        let ctx = egui::Context::default();
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while runner.running && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
            runner.poll(&ctx);
        }
        assert!(!runner.running, "child exited");
        assert!(runner.lines.iter().any(|l| l == "> xline"), "echo: {:?}", runner.lines);
        #[cfg(windows)]
        assert!(runner.lines.iter().any(|l| l == "xline"), "child response: {:?}", runner.lines);
        assert!(runner.input.is_empty(), "the input line is cleared on exit");
        assert!(runner.stdin.is_none(), "stdin is closed on exit");
    }

    /// 輸出超過行數上限時只保留尾段，並記下省略了幾行（面板據此提示）
    #[test]
    fn test_output_keeps_only_the_tail_beyond_the_line_limit() {
        let mut runner = ScriptRunner::default();
        for i in 0..(MAX_OUTPUT_LINES + 1_000) {
            runner.push_line(format!("line {i}"));
        }
        assert!(runner.lines.len() <= MAX_OUTPUT_LINES, "kept {} lines", runner.lines.len());
        assert!(runner.trimmed_lines >= 1_000, "trimmed {}", runner.trimmed_lines);
        // 最後一行一定還在，最前面那批已經被丟掉
        assert_eq!(runner.lines.last().map(String::as_str), Some(&*format!("line {}", MAX_OUTPUT_LINES + 999)));
        assert!(!runner.lines.iter().any(|l| l == "line 0"), "the oldest lines are dropped");
        // 全文與行清單必須一致（複製、選取都依賴這點）
        assert_eq!(runner.text, runner.lines.join("\n"));
    }

    /// 位元組上限：幾行超長文字也會被裁到預算內；單行超長只保留結尾
    #[test]
    fn test_output_is_capped_by_bytes_even_for_one_huge_line() {
        let mut runner = ScriptRunner::default();
        let chunk = "x".repeat(MAX_OUTPUT_CHARS / 2 + 10);
        runner.push_line(chunk.clone());
        runner.push_line(chunk.clone());
        assert!(runner.text.len() <= MAX_OUTPUT_CHARS, "text {} bytes", runner.text.len());
        assert_eq!(runner.text, runner.lines.join("\n"));
        // 單行本身超過預算：保留結尾並加省略標記
        let mut one = ScriptRunner::default();
        one.push_line("y".repeat(MAX_OUTPUT_CHARS * 2));
        assert!(one.text.len() <= MAX_OUTPUT_CHARS, "one line {} bytes", one.text.len());
        assert!(one.text.starts_with('…'), "the truncated head is marked");
        assert_eq!(one.trimmed_lines, 1);
    }

    /// 清空輸出要一併清掉截斷提示
    #[test]
    fn test_clear_output_resets_the_trim_notice() {
        let mut runner = ScriptRunner::default();
        for i in 0..(MAX_OUTPUT_LINES + 1) {
            runner.push_line(format!("line {i}"));
        }
        assert!(runner.trimmed_lines > 0);
        runner.clear_output();
        assert_eq!(runner.trimmed_lines, 0);
        assert!(runner.text.is_empty() && runner.lines.is_empty());
    }

    /// 「輸出到檔案」：記錄檔收下每一行（不受面板截斷限制）；勾選開啟時以面板目前內容
    /// 清空重寫（兩邊同步），新的 F5 運行從頭寫，停用後不再寫入；清空面板不影響檔案。
    #[test]
    fn test_log_file_receives_every_line_and_is_truncated_on_new_run() {
        let dir = std::env::temp_dir().join(format!("madedit-runner-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("result.log");
        let mut runner = ScriptRunner::default();
        runner.log_path = Some(log.clone());
        // 勾選開啟前面板已有內容：開啟時整段寫入（同步）
        runner.push_line("before".into());
        runner.set_log_enabled(true).expect("open log");
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "before\n", "enabling syncs the file with the panel");
        // 超過面板上限的輸出，記錄檔必須一行不少
        for i in 0..(MAX_OUTPUT_LINES + 300) {
            runner.push_line(format!("line {i}"));
        }
        assert!(runner.lines.len() <= MAX_OUTPUT_LINES, "panel stays capped");
        assert_eq!(std::fs::read_to_string(&log).unwrap().lines().count(), MAX_OUTPUT_LINES + 301, "the log keeps every line");
        // 新運行（F5）：從頭寫
        runner.reopen_log_truncated().expect("truncate");
        runner.push_line("fresh".into());
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "fresh\n");
        // 停用後不再寫入
        runner.set_log_enabled(false).expect("close log");
        runner.push_line("dropped".into());
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "fresh\n");
        // 面板「清空」不影響檔案
        runner.set_log_enabled(true).expect("reopen");
        runner.reopen_log_truncated().expect("truncate");
        runner.push_line("kept".into());
        runner.clear_output();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "kept\n");
        runner.disable_log();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 記錄檔預設跟著腳本走：腳本所在目錄／主名.result（log_path 覆寫優先；無腳本退回 exe 目錄）
    #[test]
    fn test_log_path_defaults_next_to_the_script() {
        let mut runner = ScriptRunner::default();
        runner.script_path = Some(PathBuf::from(r"D:\work\demo\a.nut"));
        assert_eq!(runner.resolved_log_path(), Some(PathBuf::from(r"D:\work\demo\a.nut.result")));
        // 同目錄同名不同副檔名：互不覆蓋
        runner.script_path = Some(PathBuf::from(r"D:\work\demo\a.py"));
        assert_eq!(runner.resolved_log_path(), Some(PathBuf::from(r"D:\work\demo\a.py.result")));
        // ztr 配置等多段副檔名：整個檔名後直接追加
        runner.script_path = Some(PathBuf::from("/opt/cfg/run.ztrtoml"));
        assert_eq!(runner.resolved_log_path(), Some(PathBuf::from("/opt/cfg/run.ztrtoml.result")));
        // 沒有腳本路徑：退回 MadEdit.exe 同目錄
        let runner = ScriptRunner::default();
        let default = runner.resolved_log_path().expect("a default path");
        assert_eq!(default.file_name().map(|n| n.to_string_lossy().into_owned()), Some("result.log".into()));
        let mut overridden = ScriptRunner::default();
        overridden.log_path = Some(PathBuf::from("/tmp/custom.log"));
        assert_eq!(overridden.resolved_log_path(), Some(PathBuf::from("/tmp/custom.log")));
    }
}
