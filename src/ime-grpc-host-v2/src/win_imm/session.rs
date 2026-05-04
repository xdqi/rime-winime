use std::sync::Once;
use windows::core::w;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::Ime::{ImmAssociateContextEx, IACE_DEFAULT};
use windows::Win32::UI::Input::Ime::{
    ImmAssociateContext, ImmCreateContext, ImmDestroyContext, HIMC,
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SetForegroundWindow,
    SetWindowPos, ShowWindow, HMENU, HWND_TOPMOST, SWP_SHOWWINDOW, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSW, WS_CHILD, WS_VISIBLE,
};

unsafe extern "system" fn default_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

static REGISTER_CLASS: Once = Once::new();

fn ensure_window_class() {
    REGISTER_CLASS.call_once(|| {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(default_wndproc),
            lpszClassName: w!("ImeHostWindow"),
            ..Default::default()
        };
        unsafe {
            RegisterClassW(&wc);
        }
    });
}

pub struct WinImmSession {
    pub session_id: usize,
    pub hwnd: HWND,
    pub target_hwnd: HWND,
    pub himc: HIMC,
    pub h_ime_module: HMODULE,
    pub pending_commit: Option<String>,
}

impl WinImmSession {
    pub fn create(
        session_id: usize,
        h_ime_module: HMODULE,
        show_window: bool,
    ) -> Result<Self, windows::core::Error> {
        ensure_window_class();

        unsafe {
            let _ = show_window;
            let style = if show_window {
                WS_VISIBLE
            } else {
                WINDOW_STYLE(0)
            };

            let hwnd = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("ImeHostWindow"),
                w!("HiddenImeWindow"),
                style,
                0,
                0,
                800,
                600,
                HWND(0 as _),
                None,
                None,
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(err) => {
                    tracing::error!(
                        "WinImmSession::create: CreateWindowExW failed: {:?}",
                        err
                    );
                    return Err(err);
                }
            };

            let target_hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                8,
                8,
                hwnd,
                HMENU(0 as _),
                None,
                None,
            )
            .unwrap_or(hwnd);

            let himc = ImmCreateContext();

            // ImmAssociateContext returns the previous HIMC and sets the
            // INPUTCONTEXT.hWnd — needed for ImmGenerateMessage to know
            // which window to SendMessage to.
            let _ = ImmAssociateContext(target_hwnd, himc);
            let _ = ImmAssociateContext(hwnd, himc);
            let _ = ImmAssociateContextEx(target_hwnd, himc, 0);
            let _ = ImmAssociateContextEx(hwnd, himc, 0);

            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, -32000, -32000, 8, 8, SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(target_hwnd);

            tracing::info!(
                "WinImmSession::create: session={} hwnd=0x{:X} target=0x{:X} himc=0x{:X} show_window={}",
                session_id,
                hwnd.0 as usize,
                target_hwnd.0 as usize,
                himc.0 as usize,
                show_window
            );

            Ok(Self {
                session_id,
                hwnd,
                target_hwnd,
                himc,
                h_ime_module,
                pending_commit: None,
            })
        }
    }

    pub fn destroy(&self) {
        unsafe {
            let _ = ImmAssociateContextEx(self.target_hwnd, HIMC::default(), IACE_DEFAULT);
            if self.target_hwnd != self.hwnd {
                let _ = DestroyWindow(self.target_hwnd);
            }
            // Note: Caller is responsible for calling ImeSelect(himc, FALSE) before this
            let _ = ImmDestroyContext(self.himc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// Win32 handles are essentially integer IDs, so it is safe to Send and Sync them,
// provided we respect Win32 thread affinity rules where applicable (e.g., pumping messages on the thread that created the HWND).
unsafe impl Send for WinImmSession {}
unsafe impl Sync for WinImmSession {}
