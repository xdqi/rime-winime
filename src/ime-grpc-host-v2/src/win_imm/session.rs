
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::UI::Input::Ime::{HIMC, ImmCreateContext, ImmAssociateContextEx, ImmDestroyContext, IACE_CHILDREN};
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WS_VISIBLE};
use windows::core::w;

pub struct WinImmSession {
    pub session_id: usize,
    pub hwnd: HWND,
    pub himc: HIMC,
    pub h_ime_module: HMODULE,
    pub pending_commit: Option<String>,
}

impl WinImmSession {
    pub fn create(session_id: usize, h_ime_module: HMODULE, show_window: bool) -> Result<Self, windows::core::Error> {
        unsafe {
            // Create a window
            let style = if show_window { WS_VISIBLE } else { WINDOW_STYLE(0) };
            let parent = if show_window { HWND(0 as _) } else { HWND_MESSAGE };
            
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"), // Use a simple built-in class
                w!("HiddenImeWindow"),
                style,
                0, 0, 800, 600,
                parent,
                None,
                None,
                None,
            )?;

            // Create IME context
            let himc = ImmCreateContext();
            
            // Bind context to window
            let _ = ImmAssociateContextEx(hwnd, himc, IACE_CHILDREN);

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

