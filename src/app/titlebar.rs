//! Windows 自繪標題欄的原生層。
//!
//! 主視窗以 `with_decorations(false)` 建立後，winit 對整個客戶區回報 `HTCLIENT`，
//! 視窗邊緣的縮放行為會失效。本模組用 `SetWindowSubclass` 攔截 `WM_NCHITTEST`，
//! 在視窗未最大化時把邊緣命中回報為 `HTLEFT`..`HTBOTTOMRIGHT` 縮放代碼。
//!
//! 標題列的拖動與視窗按鈕由 egui 層處理（`ViewportCommand::StartDrag` /
//! `Minimized` / `Maximized` / `Close`），不需原生命中代碼。
//!
//! 參考實作：MTT-File-Manager-RUST 的 `window_subclass.rs`（邊框命中）、
//! egui 官方 `custom_window_frame` 範例（StartDrag 拖動）。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCLIENT, HTLEFT,
    HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, WM_NCDESTROY, WM_NCHITTEST,
};

/// 縮放邊框寬度（物理像素）。固定值在各種 DPI 下都容易命中。
const RESIZE_BORDER: i32 = 8;
const SUBCLASS_ID: usize = 1;

static INSTALLED: Mutex<bool> = Mutex::new(false);

use std::sync::Mutex;

/// 在主視窗上安裝子類；重複呼叫無副作用。
pub fn install(hwnd: isize) {
    let mut installed = INSTALLED.lock().unwrap();
    if *installed {
        return;
    }
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: hwnd 是當前進程的有效視窗；回呼與 ID 都是本模組私有的。
    let ok = unsafe { SetWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID, 0) }.as_bool();
    if ok {
        *installed = true;
    }
}

fn handle_hit_test(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    // WM_NCHITTEST 的 lparam 是「螢幕座標」實體像素，低 16 位 x、高 16 位 y（可為負）。
    let mut x = (lparam.0 as i32) & 0xFFFF;
    let mut y = ((lparam.0 as i32) >> 16) & 0xFFFF;
    if x > 32767 {
        x -= 65536;
    }
    if y > 32767 {
        y -= 65536;
    }

    let mut client = RECT::default();
    let mut window = RECT::default();
    // SAFETY: hwnd 有效，輸出結構體已初始化。
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err()
        || unsafe { GetWindowRect(hwnd, &mut window) }.is_err()
    {
        return LRESULT(HTCLIENT as isize);
    }

    // 轉成視窗座標（含非客戶區邊框）。
    let x = x - window.left;
    let y = y - window.top;
    let width = window.right - window.left;
    let height = window.bottom - window.top;
    if x < 0 || y < 0 || x >= width || y >= height {
        return LRESULT(HTCLIENT as isize);
    }

    // SAFETY: hwnd 有效。
    let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
    if !zoomed {
        let on_left = x < RESIZE_BORDER;
        let on_right = x >= width - RESIZE_BORDER;
        let on_top = y < RESIZE_BORDER;
        let on_bottom = y >= height - RESIZE_BORDER;
        let hit = if on_top && on_left {
            Some(HTTOPLEFT)
        } else if on_top && on_right {
            Some(HTTOPRIGHT)
        } else if on_bottom && on_left {
            Some(HTBOTTOMLEFT)
        } else if on_bottom && on_right {
            Some(HTBOTTOMRIGHT)
        } else if on_left {
            Some(HTLEFT)
        } else if on_right {
            Some(HTRIGHT)
        } else if on_top {
            Some(HTTOP)
        } else if on_bottom {
            Some(HTBOTTOM)
        } else {
            None
        };
        if let Some(code) = hit {
            return LRESULT(code as isize);
        }
    }

    LRESULT(HTCLIENT as isize)
}

// SAFETY: 標準視窗程序簽名；只處理訊息並轉交 DefSubclassProc。
unsafe extern "system" fn procedure(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => handle_hit_test(hwnd, lparam),
        WM_NCDESTROY => {
            // SAFETY: 視窗銷毀時移除自己的子類，避免殘留回呼。
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS_ID) };
            // SAFETY: 繼續交給預設處理。
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        _ => {
            // SAFETY: 繼續交給預設處理。
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
    }
}
