//! 腳本運行器：依副檔名選擇直譯器（.nut → ZSSQConsole、.py → python、.lua → lua），
//! 後台執行目前檔案（或選中的片段），輸出串流到底部「運行輸出」面板。
//!
//! Windows 主控台程式常以 GBK/ANSI 輸出，逐行解碼時先試嚴格 UTF-8，失敗則以 GBK 解碼。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
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
    Line(String),
    Done(Result<i32, String>),
}

/// 面板保留的輸出行數上限。面板每次內容變動都要重新排版整段文字，行數無限成長會讓
/// 整個 UI 越來越慢（實測 2 萬行約 45ms/幀、10 萬行約 230ms/幀），因此只保留尾段。
pub(crate) const MAX_OUTPUT_LINES: usize = 5_000;

/// 面板保留的輸出位元組上限（UTF-8 約等於排版成本；5 千行 × 40 字元 ≈ 200KB）。
const MAX_OUTPUT_CHARS: usize = 200_000;

/// 一次運行的狀態與輸出
pub struct ScriptRunner {
    pub show: bool,
    pub running: bool,
    pub lines: Vec<String>,
    /// 輸出全文（= lines 以換行相接）；供可選取的輸出文字框顯示與複製
    pub text: String,
    /// 因超過上限而被省略的行數（面板顯示提示用）
    pub trimmed_lines: usize,
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
    /// 選中片段的臨時檔案，運行結束後刪除
    temp_file: Option<PathBuf>,
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
            exit: None,
            started: None,
            elapsed: None,
            rx: None,
            child: None,
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
        self.lines.push(line.clone());
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(&line);
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

    /// 清空輸出（面板按鈕與右鍵選單共用）
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
        let mut command = Command::new(program);
        command
            .args(args)
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
        let child = Arc::new(Mutex::new(child));
        let (tx, rx) = channel::<Msg>();

        // stdout 與 stderr 型別不同：分別包裝成統一的 Read 再交給同一條讀取邏輯
        let readers: Vec<Box<dyn Read + Send>> = vec![Box::new(stdout), Box::new(stderr)];
        for pipe in readers {
            let tx = tx.clone();
            // 輸出執行緒不保留 JoinHandle：channel 斷線或程序結束後自行退出
            std::thread::spawn(move || {
                let mut reader = BufReader::new(pipe);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf) {
                        Ok(0) => break,
                        Ok(_) => {
                            if buf.last() == Some(&b'\n') {
                                buf.pop();
                                if buf.last() == Some(&b'\r') {
                                    buf.pop();
                                }
                            }
                            if buf.is_empty() {
                                continue;
                            }
                            let line = match std::str::from_utf8(&buf) {
                                Ok(text) => text.to_string(),
                                Err(_) => {
                                    let (decoded, _, _) = encoding_rs::GBK.decode(&buf);
                                    decoded.into_owned()
                                }
                            };
                            if tx.send(Msg::Line(line)).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }
        self.worker = Some({
            let child = Arc::clone(&child);
            std::thread::spawn(move || {
                // 短鎖輪詢 try_wait：絕不持鎖阻塞，否則 UI 執行緒 stop() 會死鎖
                let code = loop {
                    let polled = child.lock().ok().and_then(|mut c| c.try_wait().ok()).flatten();
                    match polled.map(|status| status.code().unwrap_or(-1)) {
                        Some(code) => break Ok(code),
                        None => std::thread::sleep(std::time::Duration::from_millis(100)),
                    }
                };
                let _ = tx.send(Msg::Done(match code {
                    Ok(code) => Ok(code),
                    Err(e) => Err(e),
                }));
            })
        });
        self.child = Some(child);
        self.pid = Some(pid);
        self.rx = Some(rx);
        self.running = true;
        self.started = Some(Instant::now());
        self.elapsed = None;
        self.exit = None;
        self.stopped = false;
        Ok(())
    }

    /// 收取子程序輸出與結束事件；背景執行緒活著時安排重繪
    pub fn poll(&mut self, ctx: &egui::Context) {
        // 先取出接收端，才能在同一輪迴圈裡同時收訊息與改動自身
        let Some(rx) = self.rx.take() else { return };
        let mut keep_rx = true;
        loop {
            match rx.try_recv() {
                Ok(Msg::Line(line)) => self.push_line(line),
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
                    if let Some(tmp) = self.temp_file.take() {
                        let _ = std::fs::remove_file(tmp);
                    }
                    if let Some(worker) = self.worker.take() {
                        let _ = worker.join();
                    }
                    self.child = None;
                    self.pid = None;
                    ctx.request_repaint();
                    // 結束後不再放回接收端
                    keep_rx = false;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.running = false;
                    keep_rx = false;
                    break;
                }
            }
        }
        if keep_rx {
            self.rx = Some(rx);
        }
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
pub fn write_selection_temp(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("madedit_run_{name}"));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// 依副檔名選擇直譯器。`.nut` 優先使用 MadEdit.exe 同目錄的 ZSSQConsole.exe（如用戶提供的
/// Squirrel 主控台封裝），其餘 `.py`、`.lua` 依賴系統 PATH。回傳（程式、前置參數）。
pub fn interpreter_for(ext: &str) -> Option<(String, Vec<String>)> {
    match ext {
        ".nut" => {
            let exe = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("ZSSQConsole.exe")))
                .filter(|p| p.exists())
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "ZSSQConsole.exe".into());
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
            self.runner.stop();
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
                self.runner.temp_file = Some(tmp.clone());
                tmp
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
        args.push(file.display().to_string());
        let cwd = file.parent().map(|p| p.to_path_buf());
        self.runner.show = true;
        self.runner.lines.clear();
        self.runner.text.clear();
        self.runner.trimmed_lines = 0;
        if let Err(e) = self.runner.start_process(&program, &args, cwd.as_deref()) {
            // 啟動失敗時不會有 Done 事件，立即清掉暫存檔
            if let Some(tmp) = self.runner.temp_file.take() {
                let _ = std::fs::remove_file(tmp);
            }
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
        egui::Popup::menu(&body).open_memory(open).at_pointer_fixed().show(|ui| output_menu(ui, lang, &mut menu));
        if selected.is_some() {
            self.runner.sel = selected;
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
        if !has_output && !running {
            ui.painter().text(empty_center, Align2::CENTER_CENTER, self.t("run.empty", "（尚無輸出）"), output_font, ui.visuals().weak_text_color());
        }
        if running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

/// 輸出面板右鍵選單的動作（全選／複製／清空）
#[derive(Default)]
struct OutputMenu {
    select_all: bool,
    copy: bool,
    clear: bool,
}

/// 輸出面板右鍵選單；空白處與文字框共用同一份選單
fn output_menu(ui: &mut egui::Ui, lang: i18n::Lang, menu: &mut OutputMenu) {
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
        assert!(nut.to_lowercase().ends_with("zssqconsole.exe") || nut == "ZSSQConsole.exe", "{nut}");
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
    fn test_selection_temp_file_keeps_content_and_extension() {
        let bytes = "print(\"hi\");\n".as_bytes();
        let tmp = write_selection_temp("madedit_test_selection.nut", bytes).expect("write temp");
        assert_eq!(tmp.extension().and_then(|e| e.to_str()), Some("nut"));
        assert_eq!(std::fs::read(&tmp).unwrap(), bytes);
        let _ = std::fs::remove_file(tmp);
    }

    #[test]
    fn test_selection_temp_file_preserves_gbk_bytes() {
        // 選取片段沿用原檔編碼：GBK 檔案寫出的暫存檔必須仍是 GBK 位元組，
        // 否則 ZSSQConsole 之類的 ANSI 直譯器會把中文當成亂碼。
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
}
