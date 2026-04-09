use std::sync::Once;
use windows::core::w;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::Ime::{
    ImmAssociateContext, ImmCreateContext, ImmDestroyContext, HIMC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, HWND_MESSAGE, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSW, WS_VISIBLE,
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
    REGISTER_CLASS.call_once(|| unsafe {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(default_wndproc),
            lpszClassName: w!("ImeHostWindow").into(),
            hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                .unwrap_or_default()
                .into(),
            ..std::mem::zeroed()
        };
        let atom = RegisterClassW(&wc);
        if atom == 0 {
            tracing::error!("Failed to register ImeHostWindow class");
        }
    });
}

pub struct WinImmSession {
    pub session_id: usize,
    pub hwnd: HWND,
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
            let style = if show_window {
                WS_VISIBLE
            } else {
                WINDOW_STYLE(0)
            };
            let parent = if show_window {
                HWND(0 as _)
            } else {
                HWND_MESSAGE
            };

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("ImeHostWindow"),
                w!("HiddenImeWindow"),
                style,
                0,
                0,
                800,
                600,
                parent,
                None,
                None,
                None,
            )?;

            let himc = ImmCreateContext();

            // ImmAssociateContext returns the previous HIMC and sets the
            // INPUTCONTEXT.hWnd — needed for ImmGenerateMessage to know
            // which window to SendMessage to.
            let _ = ImmAssociateContext(hwnd, himc);

            Ok(Self {
                session_id,
                hwnd,
                himc,
                h_ime_module,
                pending_commit: None,
            })
        }
    }

    pub fn destroy(&self) {
        unsafe {
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
