
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::Ime::{
    HIMC, TRANSMSG, ImmCreateContext, ImmAssociateContext, ImmDestroyContext,
    ImmGetContext, ImmReleaseContext, ImmGetCompositionStringW, GCS_RESULTSTR,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DefWindowProcW, RegisterClassW, SendMessageW,
    HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE, WS_VISIBLE, WNDCLASSW,
};
use windows::core::w;
use std::sync::Once;

const WM_IME_COMPOSITION: u32 = 0x010F;
const WM_IME_CHAR: u32 = 0x0286;
const WM_CHAR: u32 = 0x0102;
/// SogouPY Win8+ custom message: wparam=count, lparam=ptr to TRANSMSG[]
const WM_SOGOU_TRANSMSG: u32 = 0x8BB8;

/// Read GCS_RESULTSTR from the HIMC associated with `hwnd` and send
/// WM_IME_CHAR per character. Called synchronously while composition
/// string is still populated.
unsafe fn dispatch_result_string(hwnd: HWND) {
    let himc = ImmGetContext(hwnd);
    if himc.is_invalid() { return; }
    let size = ImmGetCompositionStringW(himc, GCS_RESULTSTR, None, 0);
    if size > 0 {
        let wchar_count = size as usize / 2;
        let mut buf: Vec<u16> = vec![0; wchar_count];
        ImmGetCompositionStringW(
            himc,
            GCS_RESULTSTR,
            Some(buf.as_mut_ptr() as *mut _),
            size as u32,
        );
        for &ch in &buf {
            SendMessageW(hwnd, WM_IME_CHAR, WPARAM(ch as usize), LPARAM(1));
        }
    }
    let _ = ImmReleaseContext(hwnd, himc);
}

/// Custom WndProc that intercepts IME messages synchronously.
///
/// Handles two modes:
/// - Win7 path (ImmGenerateMessage): receives WM_IME_COMPOSITION directly
/// - Win8+ path (0x8BB8 custom msg): processes embedded TRANSMSG array
///
/// In both cases reads GCS_RESULTSTR while the buffer is still populated,
/// then posts WM_CHAR for each character (matching nt5src DefWindowProcW).
unsafe extern "system" fn ime_host_wndproc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    // Win7 path: ImmGenerateMessage dispatches these directly
    if msg == WM_IME_COMPOSITION && (lparam.0 as u32 & 0x0800) != 0 {
        dispatch_result_string(hwnd);
        return LRESULT(0);
    }

    if msg == WM_IME_CHAR {
        windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd, WM_CHAR, wparam, LPARAM(1),
        ).ok();
        return LRESULT(0);
    }

    // Win8+ path: SogouPY sends TRANSMSGs via custom 0x8BB8 message
    if msg == WM_SOGOU_TRANSMSG {
        let count = wparam.0;
        let ptr = lparam.0 as *const TRANSMSG;
        if !ptr.is_null() && count > 0 {
            for i in 0..count {
                let tmsg = &*ptr.add(i);
                if tmsg.message == WM_IME_COMPOSITION
                    && (tmsg.lParam.0 as u32 & 0x0800) != 0
                {
                    dispatch_result_string(hwnd);
                }
                if tmsg.message == WM_IME_CHAR {
                    windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                        hwnd, WM_CHAR, tmsg.wParam, LPARAM(1),
                    ).ok();
                }
            }
        }
        return LRESULT(count as isize);
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

static REGISTER_CLASS: Once = Once::new();

fn ensure_window_class() {
    REGISTER_CLASS.call_once(|| {
        unsafe {
            let wc = WNDCLASSW {
                lpfnWndProc: Some(ime_host_wndproc),
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
    pub fn create(session_id: usize, h_ime_module: HMODULE, show_window: bool) -> Result<Self, windows::core::Error> {
        ensure_window_class();
        unsafe {
            let style = if show_window { WS_VISIBLE } else { WINDOW_STYLE(0) };
            let parent = if show_window { HWND(0 as _) } else { HWND_MESSAGE };
            
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("ImeHostWindow"),
                w!("HiddenImeWindow"),
                style,
                0, 0, 800, 600,
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

