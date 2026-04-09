use super::{
    BackendCandidate, BackendCommitResult, BackendEventResult, BackendKeyEvent,
    BackendQueryResult, BackendSnapshot, ImeBackend,
};

use std::env;
use std::cell::RefCell;
use tracing::info;

use windows::core::{s, w, Error, PCWSTR};
use windows::Win32::Foundation::{BOOL, FreeLibrary, HINSTANCE, HMODULE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Input::Ime::{
    ImmAssociateContext, ImmCreateContext, ImmDestroyContext, ImmGetCandidateListA,
    ImmGetCandidateListCountA, ImmGetCandidateListCountW, ImmGetCandidateListW,
    ImmGetCompositionStringW, ImmGetContext, ImmGetDefaultIMEWnd, ImmReleaseContext,
    ImmSetConversionStatus, ImmSetOpenStatus, CANDIDATELIST, IME_COMPOSITION_STRING,
    IME_SENTENCE_MODE, IME_CMODE_NATIVE, GCS_COMPREADSTR, GCS_COMPSTR, HIMC,
    IMN_SETCONVERSIONMODE, IMN_SETOPENSTATUS, ISC_SHOWUIALL,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ActivateKeyboardLayout, GetKeyboardLayout, GetKeyboardState, LoadKeyboardLayoutW,
    MapVirtualKeyW, ToAsciiEx, ToUnicode, VkKeyScanExW, ACTIVATE_KEYBOARD_LAYOUT_FLAGS,
    KLF_ACTIVATE, KLF_REORDER, KLF_SUBSTITUTE_OK, MAPVK_VK_TO_VSC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowThreadProcessId, PeekMessageW,
    SendMessageW, SetForegroundWindow, SetWindowPos, ShowWindow, TranslateMessage,
    HMENU, HWND_TOPMOST, MSG, PM_REMOVE, SWP_SHOWWINDOW, SW_SHOW, WINDOW_EX_STYLE,
    WM_IME_NOTIFY, WM_IME_SELECT, WM_IME_SETCONTEXT, WM_INPUTLANGCHANGE,
    WM_INPUTLANGCHANGEREQUEST, WM_KEYDOWN, WM_KEYUP, WS_CHILD, WS_VISIBLE,
};

#[derive(Debug)]
struct WinImmRuntime {
    hwnd_raw: isize,
    target_hwnd_raw: isize,
    himc_raw: isize,
    previous_himc_raw: isize,
}

#[derive(Debug, Clone)]
struct ImmCandidatePage {
    candidates: Vec<BackendCandidate>,
    selected_index: u32,
    page_size: u32,
}

const IPHK_PROCESSBYIME_FLAG: u32 = 0x0002;

type FnImeProcessKey = unsafe extern "system" fn(HIMC, u32, isize, *const u8) -> BOOL;
type FnImeToAsciiEx = unsafe extern "system" fn(u32, u32, *const u8, *mut core::ffi::c_void, u32, HIMC) -> u32;
type FnImeSelect = unsafe extern "system" fn(HIMC, BOOL) -> BOOL;
type FnImeSetActiveContext = unsafe extern "system" fn(HIMC, BOOL) -> BOOL;
type FnImmProcessKey = unsafe extern "system" fn(HWND, isize, u32, isize, u32) -> u32;
type FnImmTranslateMessage = unsafe extern "system" fn(HWND, u32, usize, isize) -> BOOL;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TransMsgCompat {
    message: u32,
    w_param: usize,
    l_param: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
#[derive(Default)]
struct TransMsgListCompat {
    u_msg_count: u32,
    trans_msg: [TransMsgCompat; 16],
}

#[derive(Debug)]
struct QqImeExports {
    module_raw: isize,
    ime_process_key: FnImeProcessKey,
    ime_to_ascii_ex: FnImeToAsciiEx,
    ime_select: Option<FnImeSelect>,
    ime_set_active_context: Option<FnImeSetActiveContext>,
}

#[derive(Debug)]
struct Imm32Exports {
    module_raw: isize,
    imm_process_key: Option<FnImmProcessKey>,
    imm_translate_message: Option<FnImmTranslateMessage>,
}

impl QqImeExports {
    fn module(&self) -> HMODULE {
        HMODULE(self.module_raw as *mut core::ffi::c_void)
    }

    fn candidate_paths() -> Vec<String> {
        let mut out = Vec::new();

        if let Ok(path) = env::var("IME_WINIMM_DLL") {
            if !path.trim().is_empty() {
                out.push(path);
            }
        }

        out.push("C:\\windows\\system32\\QQPinyin.ime".to_string());
        out.push("C:\\windows\\system32\\SogouPY.ime".to_string());
        out.push("C:\\windows\\syswow64\\SogouPY.ime".to_string());
        out.push("Z:\\opt\\sogou\\sys\\SogouPY.ime".to_string());
        out.push("Z:\\opt\\sogou\\syswow64\\SogouPY.ime".to_string());

        out
    }

    fn load() -> Result<Self, String> {
        let mut errors = Vec::new();

        for path in Self::candidate_paths() {
            let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

            // Safety: wide is NUL-terminated and lives for the duration of call.
            let module = match unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) } {
                Ok(module) => module,
                Err(err) => {
                    errors.push(format!("LoadLibraryW({path}) failed: {err}"));
                    continue;
                }
            };

            let process = unsafe { GetProcAddress(module, s!("ImeProcessKey")) };
            let to_ascii = unsafe { GetProcAddress(module, s!("ImeToAsciiEx")) };
            let select = unsafe { GetProcAddress(module, s!("ImeSelect")) };
            let set_active = unsafe { GetProcAddress(module, s!("ImeSetActiveContext")) };

            let (Some(process), Some(to_ascii)) = (process, to_ascii) else {
                // Safety: module was successfully loaded above.
                unsafe {
                    let _ = FreeLibrary(module);
                }
                errors.push(format!(
                    "GetProcAddress exports missing in {path} (ImeProcessKey/ImeToAsciiEx)"
                ));
                continue;
            };

            // Safety: function pointers originate from GetProcAddress of the
            // loaded IME module and are cast to the documented signatures.
            let ime_process_key: FnImeProcessKey = unsafe {
                core::mem::transmute::<unsafe extern "system" fn() -> isize, FnImeProcessKey>(
                    process,
                )
            };
            let ime_to_ascii_ex: FnImeToAsciiEx = unsafe {
                core::mem::transmute::<unsafe extern "system" fn() -> isize, FnImeToAsciiEx>(
                    to_ascii,
                )
            };
            let ime_select: Option<FnImeSelect> = select.map(|f| unsafe {
                core::mem::transmute::<unsafe extern "system" fn() -> isize, FnImeSelect>(f)
            });
            let ime_set_active_context: Option<FnImeSetActiveContext> = set_active.map(|f| unsafe {
                core::mem::transmute::<unsafe extern "system" fn() -> isize, FnImeSetActiveContext>(
                    f,
                )
            });

            return Ok(Self {
                module_raw: module.0 as isize,
                ime_process_key,
                ime_to_ascii_ex,
                ime_select,
                ime_set_active_context,
            });
        }

        Err(format!(
            "failed to load QQPinyin IME exports: {}",
            errors.join(" | ")
        ))
    }
}

impl Imm32Exports {
    fn module(&self) -> HMODULE {
        HMODULE(self.module_raw as *mut core::ffi::c_void)
    }

    fn load() -> Result<Self, String> {
        // Safety: static NUL-terminated UTF-16 literal.
        let module = unsafe { LoadLibraryW(w!("imm32.dll")) }
            .map_err(|err| format!("LoadLibraryW(imm32.dll) failed: {err}"))?;

        // Safety: function addresses are looked up in loaded imm32 module.
        let process = unsafe { GetProcAddress(module, s!("ImmProcessKey")) };
        // Safety: function addresses are looked up in loaded imm32 module.
        let translate = unsafe { GetProcAddress(module, s!("ImmTranslateMessage")) };

        let imm_process_key = process.map(|f| unsafe {
            core::mem::transmute::<unsafe extern "system" fn() -> isize, FnImmProcessKey>(f)
        });
        let imm_translate_message = translate.map(|f| unsafe {
            core::mem::transmute::<unsafe extern "system" fn() -> isize, FnImmTranslateMessage>(f)
        });

        Ok(Self {
            module_raw: module.0 as isize,
            imm_process_key,
            imm_translate_message,
        })
    }
}

impl Drop for QqImeExports {
    fn drop(&mut self) {
        // Safety: module handle was returned by LoadLibraryW and is released once here.
        unsafe {
            let _ = FreeLibrary(self.module());
        }
    }
}

impl Drop for Imm32Exports {
    fn drop(&mut self) {
        // Safety: module handle was returned by LoadLibraryW and is released once here.
        unsafe {
            let _ = FreeLibrary(self.module());
        }
    }
}

impl WinImmRuntime {
    fn hwnd(&self) -> HWND {
        HWND(self.hwnd_raw as *mut core::ffi::c_void)
    }

    fn has_window(&self) -> bool {
        self.hwnd_raw != 0
    }

    fn target_hwnd(&self) -> HWND {
        if self.target_hwnd_raw != 0 {
            HWND(self.target_hwnd_raw as *mut core::ffi::c_void)
        } else {
            self.hwnd()
        }
    }

    fn himc(&self) -> HIMC {
        HIMC(self.himc_raw as *mut core::ffi::c_void)
    }

    fn previous_himc(&self) -> HIMC {
        HIMC(self.previous_himc_raw as *mut core::ffi::c_void)
    }

    fn create() -> Result<Self, String> {
        // Safety: ImmCreateContext allocates an IME input context owned by
        // this process; it is released in Drop via ImmDestroyContext.
        let himc = unsafe { ImmCreateContext() };
        if himc.0.is_null() {
            return Err(format!("ImmCreateContext failed: {}", Error::from_win32()));
        }

        // Safety: Creating a hidden top-level window with predefined class
        // `STATIC` and null parent/menu/instance is a standard Win32 pattern.
        let hwnd_result = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("ime-grpc-host-hidden"),
                WS_VISIBLE,
                0,
                0,
                0,
                0,
                HWND(std::ptr::null_mut()),
                HMENU(std::ptr::null_mut()),
                HINSTANCE(std::ptr::null_mut()),
                None,
            )
        };

        match hwnd_result {
            Ok(hwnd) => {
                // Safety: Child EDIT target bound to host window, fallback to host if unavailable.
                let target_hwnd = unsafe {
                    CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        w!("EDIT"),
                        w!(""),
                        WS_CHILD | WS_VISIBLE,
                        0,
                        0,
                        8,
                        8,
                        hwnd,
                        HMENU(std::ptr::null_mut()),
                        HINSTANCE(std::ptr::null_mut()),
                        None,
                    )
                }
                .unwrap_or(hwnd);

                // Safety: Associate created HIMC with valid HWND, saving previous
                // HIMC for restoration during Drop.
                let previous_himc = unsafe { ImmAssociateContext(target_hwnd, himc) };

                // Safety: Mirror association to host window for IME routing consistency.
                unsafe {
                    let _ = ImmAssociateContext(hwnd, himc);
                }

                // Safety: Keep tiny host window off-screen while still focusable for IME.
                unsafe {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = SetWindowPos(hwnd, HWND_TOPMOST, -32000, -32000, 8, 8, SWP_SHOWWINDOW);
                    let _ = SetForegroundWindow(hwnd);
                }

                // Safety: Any HIMC acquired by ImmGetContext is released immediately.
                let active_himc = unsafe { ImmGetContext(target_hwnd) };
                if active_himc.0.is_null() {
                    // Safety: Undo partial setup in reverse order.
                    unsafe {
                        let _ = ImmAssociateContext(target_hwnd, previous_himc);
                        let _ = ImmDestroyContext(himc);
                        if target_hwnd.0 != hwnd.0 {
                            let _ = DestroyWindow(target_hwnd);
                        }
                        let _ = DestroyWindow(hwnd);
                    }
                    return Err("ImmGetContext returned null after ImmAssociateContext".to_string());
                }

                // Safety: Matches ImmGetContext above.
                unsafe {
                    let _ = ImmReleaseContext(target_hwnd, active_himc);
                }

                Ok(Self {
                    hwnd_raw: hwnd.0 as isize,
                    target_hwnd_raw: target_hwnd.0 as isize,
                    himc_raw: himc.0 as isize,
                    previous_himc_raw: previous_himc.0 as isize,
                })
            }
            Err(_) => Ok(Self {
                // Headless mode: no HWND available, but keep HIMC alive for
                // direct IME export probing.
                hwnd_raw: 0,
                target_hwnd_raw: 0,
                himc_raw: himc.0 as isize,
                previous_himc_raw: 0,
            }),
        }
    }

    fn ensure_context_alive(&self) -> Result<(), String> {
        if self.has_window() {
            // Safety: self.hwnd is owned by this runtime. Any acquired HIMC is
            // immediately released by ImmReleaseContext.
            let active_himc = unsafe { ImmGetContext(self.target_hwnd()) };
            if active_himc.0.is_null() {
                return Err("ImmGetContext returned null; IME context is not active".to_string());
            }

            // Safety: Matches ImmGetContext above.
            unsafe {
                let _ = ImmReleaseContext(self.target_hwnd(), active_himc);
            }
            return Ok(());
        }

        if self.himc().0.is_null() {
            return Err("no HIMC allocated".to_string());
        }

        Ok(())
    }
}

impl Drop for WinImmRuntime {
    fn drop(&mut self) {
        // Safety: Drop runs once and releases resources in reverse order.
        unsafe {
            if self.has_window() {
                let _ = ImmAssociateContext(self.target_hwnd(), self.previous_himc());
            }
            let _ = ImmDestroyContext(self.himc());
            if self.has_window() {
                if self.target_hwnd_raw != 0 && self.target_hwnd_raw != self.hwnd_raw {
                    let _ = DestroyWindow(self.target_hwnd());
                }
                let _ = DestroyWindow(self.hwnd());
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct WinImmBackend {
    composition: String,
    reading: String,
    candidates: Vec<BackendCandidate>,
    debug_timeline: RefCell<Vec<String>>,
    backend_state_version: u64,
    init_error: Option<String>,
        runtime: Option<WinImmRuntime>,
        ime_exports: Option<QqImeExports>,
        imm32_exports: Option<Imm32Exports>,
        force_real_imm: bool,
        trace_timeline: bool,
}

impl WinImmBackend {
    pub fn from_env() -> Self {
                {
            let force_real_imm = env::var("IME_WINIMM_FORCE_REAL")
                .ok()
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(false);
            let trace_timeline = env::var("IME_WINIMM_TRACE_TIMELINE")
                .ok()
                .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
                .unwrap_or(false);

            let (runtime, init_error) = match WinImmRuntime::create() {
                Ok(runtime) => (Some(runtime), None),
                Err(err) => (None, Some(err)),
            };

            let ime_exports = QqImeExports::load().ok();
            let imm32_exports = Imm32Exports::load().ok();
            let init_error = if init_error.is_some() {
                init_error
            } else if ime_exports.is_none() {
                Some("QQPinyin exports unavailable; will use fallback composition path".to_string())
            } else {
                None
            };

            Self {
                backend_state_version: 1,
                init_error,
                runtime,
                ime_exports,
                imm32_exports,
                force_real_imm,
                trace_timeline,
                ..Self::default()
            }
        }

        #[cfg(not(windows))]
        {
            Self {
                backend_state_version: 1,
                init_error: Some(Self::unsupported_message()),
                ..Self::default()
            }
        }
    }

    fn clear_local_state(&mut self) {
        self.composition.clear();
        self.reading.clear();
        self.candidates.clear();
    }

        fn fake_candidates(input: &str, max_count: usize) -> Vec<BackendCandidate> {
        if input.is_empty() || max_count == 0 {
            return Vec::new();
        }

        let mut templates = vec![
            input.to_string(),
            format!("{}1", input),
            format!("{}2", input),
            format!("{}3", input),
            format!("{}4", input),
        ];
        templates.dedup();

        templates
            .into_iter()
            .take(max_count)
            .enumerate()
            .map(|(idx, text)| BackendCandidate {
                index: idx as u32,
                text,
                comment: "win_imm-min".to_string(),
                quality: (100.0 - idx as f64).max(1.0),
            })
            .collect()
    }

        fn refresh_fallback_candidates(&mut self, max_candidates: usize) {
        self.reading = self.composition.clone();
        self.candidates = Self::fake_candidates(&self.composition, max_candidates);
    }

        fn timeline_log<F>(&self, stage: &'static str, detail_fn: F)
    where
        F: FnOnce() -> String,
    {
        if !self.trace_timeline {
            return;
        }

        let detail = detail_fn();
        self
            .debug_timeline
            .borrow_mut()
            .push(format!("{} {}", stage, detail));
        eprintln!("win_imm_timeline stage={} detail={}", stage, detail);
        info!(target: "win_imm_timeline", stage = %stage, detail = %detail);
    }

        fn key_lparam_from_vk(vk: u32, key_up: bool, alt_down: bool) -> isize {
        // Safety: pure keycode-to-scan conversion without memory access.
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) } as isize;
        let mut l = 1_isize | (scan << 16);

        if alt_down {
            l |= 1_isize << 29;
        }
        if key_up {
            l |= 1_isize << 30;
            l |= 1_isize << 31;
        }
        l
    }

        fn resolve_vk_and_modifiers(
        virtual_key: u32,
        hkl: windows::Win32::UI::Input::KeyboardAndMouse::HKL,
    ) -> (u32, bool, bool, bool, Option<u16>) {
        if virtual_key <= 0x7f {
            let ch = virtual_key as u8 as char;
            if ch.is_ascii_graphic() || ch == ' ' {
                // Safety: pure key translation query with provided HKL.
                let packed = unsafe { VkKeyScanExW(ch as u16, hkl) };
                if packed != -1 {
                    let packed_u = packed as u16;
                    let vk = (packed_u & 0x00ff) as u32;
                    if vk != 0 && vk != 0xff {
                        let mods = ((packed_u >> 8) & 0x00ff) as u8;
                        return (
                            vk,
                            (mods & 0x01) != 0,
                            (mods & 0x02) != 0,
                            (mods & 0x04) != 0,
                            Some(ch as u16),
                        );
                    }
                }
            }
        }

        (virtual_key, false, false, false, None)
    }

        fn send_modifier_key(target: HWND, vk: u32, key_down: bool, alt_down: bool) {
        let lparam = Self::key_lparam_from_vk(vk, !key_down, alt_down);
        // Safety: sends synthetic key message to owned target window.
        unsafe {
            let _ = SendMessageW(
                target,
                if key_down { WM_KEYDOWN } else { WM_KEYUP },
                WPARAM(vk as usize),
                LPARAM(lparam),
            );
        }
    }

        fn acquire_active_himc(&self) -> Result<(HIMC, bool), String> {
        if let Some(runtime) = self.runtime.as_ref() {
            if runtime.has_window() {
                // Safety: runtime owns this HWND and context is released by caller.
                let ctx = unsafe { ImmGetContext(runtime.target_hwnd()) };
                if !ctx.0.is_null() {
                    return Ok((ctx, true));
                }
            }

            if !runtime.himc().0.is_null() {
                return Ok((runtime.himc(), false));
            }
        }

        Err(self.not_ready_message())
    }

        fn release_active_himc(&self, himc: HIMC, borrowed_from_window: bool) {
        if !borrowed_from_window {
            return;
        }

        if let Some(runtime) = self.runtime.as_ref() {
            if runtime.has_window() {
                // Safety: Matches ImmGetContext acquisition on runtime.hwnd().
                unsafe {
                    let _ = ImmReleaseContext(runtime.target_hwnd(), himc);
                }
            }
        }
    }

        fn dispatch_ime_messages(&self, trans: &TransMsgListCompat) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        if !runtime.has_window() {
            return;
        }

        let count = trans.u_msg_count.min(16) as usize;
        for msg in &trans.trans_msg[..count] {
            // Safety: dispatching IME-generated messages to the runtime window.
            unsafe {
                let _ = SendMessageW(
                    runtime.target_hwnd(),
                    msg.message,
                    WPARAM(msg.w_param),
                    LPARAM(msg.l_param),
                );
            }
        }
    }

        fn pump_messages_once() {
        let mut msg = MSG::default();

        loop {
            // Safety: message pump runs on current thread and writes into local MSG.
            let has_msg = unsafe {
                PeekMessageW(
                    &mut msg,
                    HWND(std::ptr::null_mut()),
                    0,
                    0,
                    PM_REMOVE,
                )
            };
            if !has_msg.as_bool() {
                break;
            }

            // Safety: MSG was initialized by PeekMessageW in this loop.
            unsafe {
                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }
    }

        fn activate_qq_ime(&self) -> Result<(), String> {
        let (himc, borrowed_ctx) = self.acquire_active_himc()?;

        self.ensure_ime_layout();

        if let Some(exports) = self.ime_exports.as_ref() {
            // Safety: function pointers come from validated GetProcAddress exports.
            unsafe {
                if let Some(select) = exports.ime_select {
                    let _ = select(himc, BOOL(1));
                }
                if let Some(set_active) = exports.ime_set_active_context {
                    let _ = set_active(himc, BOOL(1));
                }
            }
        }

        // Safety: himc comes from active context acquisition above.
        unsafe {
            let _ = ImmSetOpenStatus(himc, true);
            let _ = ImmSetConversionStatus(himc, IME_CMODE_NATIVE, IME_SENTENCE_MODE(0));
        }

        if let Some(runtime) = self.runtime.as_ref() {
            if runtime.has_window() {
                // Safety: runtime HWND is valid for the backend lifetime.
                unsafe {
                    let _ = SendMessageW(
                        runtime.target_hwnd(),
                        WM_IME_SETCONTEXT,
                        WPARAM(1),
                        LPARAM(ISC_SHOWUIALL as isize),
                    );

                    let def_ime = ImmGetDefaultIMEWnd(runtime.target_hwnd());
                    if !def_ime.0.is_null() && def_ime.0 != runtime.target_hwnd().0 {
                        let _ = SendMessageW(
                            def_ime,
                            WM_IME_SETCONTEXT,
                            WPARAM(1),
                            LPARAM(ISC_SHOWUIALL as isize),
                        );
                        let _ = SendMessageW(
                            def_ime,
                            WM_IME_SELECT,
                            WPARAM(1),
                            LPARAM(himc.0 as isize),
                        );
                        let _ = SendMessageW(
                            def_ime,
                            WM_IME_NOTIFY,
                            WPARAM(IMN_SETOPENSTATUS as usize),
                            LPARAM(0),
                        );
                        let _ = SendMessageW(
                            def_ime,
                            WM_IME_NOTIFY,
                            WPARAM(IMN_SETCONVERSIONMODE as usize),
                            LPARAM(0),
                        );
                    }
                }
            }
        }

        Self::pump_messages_once();
        self.timeline_log("S1_ACTIVATE_CONTEXT", || {
            let (has_window, target_hwnd_raw) = self
                .runtime
                .as_ref()
                .map(|runtime| (runtime.has_window(), runtime.target_hwnd().0 as usize))
                .unwrap_or((false, 0));

            format!(
                "himc=0x{:x} has_window={} target_hwnd=0x{:x}",
                himc.0 as usize,
                has_window,
                target_hwnd_raw
            )
        });
        self.release_active_himc(himc, borrowed_ctx);
        Ok(())
    }

        fn ensure_ime_layout(&self) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        if !runtime.has_window() {
            return;
        }

        let target = runtime.target_hwnd();
        let mut chosen = unsafe { GetKeyboardLayout(0) };
        let layout_ids = ["E0200804", "E0220804", "E0010804", "00000804", "E0200409"];
        let flags = ACTIVATE_KEYBOARD_LAYOUT_FLAGS(
            KLF_ACTIVATE.0 | KLF_SUBSTITUTE_OK.0 | KLF_REORDER.0,
        );

        for layout in layout_ids {
            let wide: Vec<u16> = layout.encode_utf16().chain(std::iter::once(0)).collect();
            // Safety: wide string is NUL-terminated and valid for the duration of this call.
            let loaded = unsafe { LoadKeyboardLayoutW(PCWSTR(wide.as_ptr()), flags) };
            if let Ok(loaded) = loaded {
                if !loaded.0.is_null() {
                    chosen = loaded;
                    break;
                }
            }
        }

        if chosen.0.is_null() {
            return;
        }

        // Safety: keyboard layout and target HWND belong to current process thread context.
        unsafe {
            let _ = ActivateKeyboardLayout(chosen, ACTIVATE_KEYBOARD_LAYOUT_FLAGS(0));
            let _ = SendMessageW(
                target,
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(chosen.0 as isize),
            );
            let _ = SendMessageW(
                target,
                WM_INPUTLANGCHANGE,
                WPARAM(0),
                LPARAM(chosen.0 as isize),
            );
        }
    }

        fn drive_qq_ime_key(&self, key_event: &BackendKeyEvent) -> Result<(), String> {
        let Some(exports) = self.ime_exports.as_ref() else {
            return Ok(());
        };

        let key_down = key_event.key_down;
        let mut virtual_key = key_event.virtual_key;
        if virtual_key == 0 && key_event.source_keycode <= 0xFF {
            virtual_key = key_event.source_keycode;
        }
        if virtual_key == 0 {
            return Ok(());
        }

        let (himc, borrowed_ctx) = self.acquire_active_himc()?;

        let mut key_state = [0_u8; 256];
        // Safety: writes into a fixed-size local key-state buffer.
        let _ = unsafe { GetKeyboardState(&mut key_state) };

        // Safety: querying current thread keyboard layout.
        let mut hkl = unsafe { GetKeyboardLayout(0) };
        let mut target_hwnd = HWND(std::ptr::null_mut());

        if let Some(runtime) = self.runtime.as_ref() {
            if runtime.has_window() {
                let target = runtime.target_hwnd();
                target_hwnd = target;
                // Safety: thread id query against owned target window handle.
                let thread_id = unsafe { GetWindowThreadProcessId(target, None) };
                if thread_id != 0 {
                    // Safety: reading keyboard layout for discovered thread id.
                    hkl = unsafe { GetKeyboardLayout(thread_id) };
                }
            }
        }

        let (resolved_vk, mapped_shift, mapped_ctrl, mapped_alt, wm_char_guess) =
            Self::resolve_vk_and_modifiers(virtual_key, hkl);

        let need_shift = key_event.shift || mapped_shift;
        let need_ctrl = key_event.ctrl || mapped_ctrl;
        let need_alt = key_event.alt || mapped_alt;
        // Safety: pure keycode-to-scan conversion without memory access.
        let scan = if key_event.scan_code != 0 {
            key_event.scan_code
        } else {
            unsafe { MapVirtualKeyW(resolved_vk, MAPVK_VK_TO_VSC) }
        };

        self.timeline_log("S2_KEY_ENTRY", || {
            format!(
                "key_down={} vk_in={} vk_resolved={} scan={} shift={} ctrl={} alt={} source_keycode={} source_modifier=0x{:x}",
                key_down,
                virtual_key,
                resolved_vk,
                scan,
                need_shift,
                need_ctrl,
                need_alt,
                key_event.source_keycode,
                key_event.source_modifier
            )
        });

        let mut wm_char = wm_char_guess;
        if wm_char.is_none() {
            let source_low = (key_event.source_keycode & 0xFF) as u8;
            if (0x20..0x7F).contains(&source_low) {
                wm_char = Some(source_low as u16);
            }
        }

        key_state[(resolved_vk & 0xFF) as usize] |= 0x80;
        if need_shift {
            key_state[0x10] |= 0x80;
        }
        if need_ctrl {
            key_state[0x11] |= 0x80;
        }
        if need_alt {
            key_state[0x12] |= 0x80;
        }

        let lparam = Self::key_lparam_from_vk(resolved_vk, !key_down, need_alt);

        if key_down && !target_hwnd.0.is_null() {
            let mut imm_translated = false;
            if need_ctrl {
                Self::send_modifier_key(target_hwnd, 0x11, true, need_alt);
            }
            if need_shift {
                Self::send_modifier_key(target_hwnd, 0x10, true, need_alt);
            }
            if need_alt {
                Self::send_modifier_key(target_hwnd, 0x12, true, true);
            }

            // Safety: sending synthetic keydown to the backend-owned window.
            unsafe {
                let _ = SendMessageW(
                    target_hwnd,
                    WM_KEYDOWN,
                    WPARAM(resolved_vk as usize),
                    LPARAM(lparam),
                );
            }

            let mut imm_flags = 0_u32;
            if let Some(imm32) = self.imm32_exports.as_ref() {
                if let Some(imm_process_key) = imm32.imm_process_key {
                    // Safety: function pointer resolved from imm32.dll.
                    imm_flags = unsafe {
                        imm_process_key(
                            target_hwnd,
                            hkl.0 as isize,
                            resolved_vk,
                            lparam,
                            u32::MAX,
                        )
                    };
                }

                if (imm_flags & IPHK_PROCESSBYIME_FLAG) != 0 {
                    if let Some(imm_translate_message) = imm32.imm_translate_message {
                        // Safety: function pointer resolved from imm32.dll.
                        unsafe {
                            let _ = imm_translate_message(
                                target_hwnd,
                                WM_KEYDOWN,
                                resolved_vk as usize,
                                lparam,
                            );
                        }
                        imm_translated = true;
                    }
                }
            }

            self.timeline_log("S3_IMM_PROCESS_TRANSLATE", || {
                format!(
                    "vk={} lparam=0x{:x} imm_flags=0x{:x} process_by_ime={} translated={}",
                    resolved_vk,
                    lparam as usize,
                    imm_flags,
                    (imm_flags & IPHK_PROCESSBYIME_FLAG) != 0,
                    imm_translated
                )
            });

            Self::pump_messages_once();
        }

        let mut ime_vk = resolved_vk;
        let mut unicode_buf = [0_u16; 1];
        // Safety: reads only from local fixed-size buffers.
        let uni_n = unsafe { ToUnicode(resolved_vk, scan, Some(&key_state), &mut unicode_buf, 0) };
        if uni_n == 1 {
            ime_vk = (ime_vk & 0x00ff) | ((unicode_buf[0] as u32) << 16);
        } else {
            let mut ascii_word: u16 = 0;
            // Safety: reads key state and writes one WORD into ascii_word.
            let asc_n = unsafe {
                ToAsciiEx(
                    resolved_vk,
                    scan,
                    Some(&key_state),
                    &mut ascii_word,
                    0,
                    hkl,
                )
            };
            if asc_n > 0 {
                ime_vk = (ime_vk & 0x00ff) | ((ascii_word as u32) << 8);
                ime_vk &= 0xffff;
            }
        }

        let mut trans = TransMsgListCompat::default();

        // Safety: function pointers come from validated GetProcAddress exports.
        let (ime_process_result, ime_to_ascii_result) = unsafe {
            let process = (exports.ime_process_key)(himc, resolved_vk, lparam, key_state.as_ptr());
            let to_ascii = (exports.ime_to_ascii_ex)(
                ime_vk,
                scan,
                key_state.as_ptr(),
                (&mut trans as *mut TransMsgListCompat).cast::<core::ffi::c_void>(),
                0,
                himc,
            );
            (process, to_ascii)
        };

        self.timeline_log("S4_IME_EXPORT_CALLS", || {
            format!(
                "ime_process_key={} ime_to_ascii_ex={} trans_msg_count={}",
                ime_process_result.as_bool(),
                ime_to_ascii_result,
                trans.u_msg_count
            )
        });

        self.dispatch_ime_messages(&trans);

        if key_down && !target_hwnd.0.is_null() {
            let key_up_lparam = Self::key_lparam_from_vk(resolved_vk, true, need_alt);
            // Safety: sending synthetic keyup to complete one key stroke.
            unsafe {
                let _ = SendMessageW(
                    target_hwnd,
                    WM_KEYUP,
                    WPARAM(resolved_vk as usize),
                    LPARAM(key_up_lparam),
                );
                if let Some(ch) = wm_char {
                    let _ = SendMessageW(target_hwnd, windows::Win32::UI::WindowsAndMessaging::WM_CHAR, WPARAM(ch as usize), LPARAM(1));
                }
            }

            if need_alt {
                Self::send_modifier_key(target_hwnd, 0x12, false, true);
            }
            if need_shift {
                Self::send_modifier_key(target_hwnd, 0x10, false, need_alt);
            }
            if need_ctrl {
                Self::send_modifier_key(target_hwnd, 0x11, false, need_alt);
            }
        }

        self.timeline_log("S5_DISPATCH_COMPLETE", || {
            format!(
                "trans_msg_count={} key_down={} wm_char={}",
                trans.u_msg_count,
                key_down,
                wm_char.unwrap_or_default()
            )
        });

        Self::pump_messages_once();
        self.release_active_himc(himc, borrowed_ctx);
        Ok(())
    }

        fn read_utf16_z_at(bytes: &[u8], offset: usize) -> String {
        if offset >= bytes.len() {
            return String::new();
        }

        let mut wide = Vec::new();
        let mut pos = offset;
        while pos + 1 < bytes.len() {
            let code = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
            if code == 0 {
                break;
            }
            wide.push(code);
            pos += 2;
        }

        String::from_utf16_lossy(&wide)
    }

        fn read_ansi_z_at(bytes: &[u8], offset: usize) -> String {
        if offset >= bytes.len() {
            return String::new();
        }

        let end = bytes[offset..]
            .iter()
            .position(|b| *b == 0)
            .map(|delta| offset + delta)
            .unwrap_or(bytes.len());

        if end <= offset {
            return String::new();
        }

        String::from_utf8_lossy(&bytes[offset..end]).into_owned()
    }

        fn read_imm_string(himc: HIMC, kind: IME_COMPOSITION_STRING) -> Result<String, String> {
        // Safety: Querying required buffer size for the same HIMC and kind.
        let byte_len = unsafe { ImmGetCompositionStringW(himc, kind, None, 0) };
        if byte_len < 0 {
            return Err(format!(
                "ImmGetCompositionStringW size query failed: kind={} err={}",
                kind.0,
                Error::from_win32()
            ));
        }
        if byte_len == 0 {
            return Ok(String::new());
        }

        let mut bytes = vec![0_u8; byte_len as usize];
        // Safety: Buffer is valid and mutable for `byte_len` bytes.
        let copied = unsafe {
            ImmGetCompositionStringW(
                himc,
                kind,
                Some(bytes.as_mut_ptr() as *mut core::ffi::c_void),
                byte_len as u32,
            )
        };
        if copied < 0 {
            return Err(format!(
                "ImmGetCompositionStringW read failed: kind={} err={}",
                kind.0,
                Error::from_win32()
            ));
        }

        let byte_count = copied as usize;
        let wide_count = byte_count / 2;
        let mut wide = Vec::with_capacity(wide_count);
        for idx in 0..wide_count {
            let lo = bytes[idx * 2];
            let hi = bytes[idx * 2 + 1];
            wide.push(u16::from_le_bytes([lo, hi]));
        }

        while matches!(wide.last(), Some(0)) {
            wide.pop();
        }

        Ok(String::from_utf16_lossy(&wide))
    }

        fn read_imm_candidates(
        himc: HIMC,
        max_candidates: usize,
    ) -> Result<ImmCandidatePage, String> {
        let mut list_count = 0_u32;
        // Safety: himc is valid for this call; output pointer targets local variable.
        let _ = unsafe { ImmGetCandidateListCountW(himc, &mut list_count as *mut u32) };

        if list_count == 0 {
            // Safety: ANSI fallback for IMEs that expose candidate buffers via A APIs.
            let _ = unsafe { ImmGetCandidateListCountA(himc, &mut list_count as *mut u32) };
        }

        if list_count == 0 {
            list_count = 1;
        }

        let requested = max_candidates.max(1);

        for list_index in 0..list_count {
            // Safety: size query for candidate list index in wide path first.
            let required_w = unsafe { ImmGetCandidateListW(himc, list_index, None, 0) };
            let (required, use_ansi) = if required_w != 0 {
                (required_w, false)
            } else {
                // Safety: ANSI fallback query for IMEs exposing non-Unicode candidate lists.
                let required_a = unsafe { ImmGetCandidateListA(himc, list_index, None, 0) };
                if required_a == 0 {
                    continue;
                }
                (required_a, true)
            };

            let mut bytes = vec![0_u8; required as usize];
            // Safety: buffer is writable for required bytes and CANDIDATELIST-compatible.
            let written = unsafe {
                if use_ansi {
                    ImmGetCandidateListA(
                        himc,
                        list_index,
                        Some(bytes.as_mut_ptr() as *mut CANDIDATELIST),
                        required,
                    )
                } else {
                    ImmGetCandidateListW(
                        himc,
                        list_index,
                        Some(bytes.as_mut_ptr() as *mut CANDIDATELIST),
                        required,
                    )
                }
            };
            if written == 0 {
                continue;
            }

            // Safety: bytes was filled by ImmGetCandidateListW with CANDIDATELIST.
            let header = unsafe { &*(bytes.as_ptr() as *const CANDIDATELIST) };
            let count = header.dwCount as usize;
            let base_page_size = if header.dwPageSize == 0 {
                requested
            } else {
                header.dwPageSize as usize
            };

            let page_start = (header.dwPageStart as usize).min(count);
            let effective_page_size = base_page_size.min(requested);
            let page_end = page_start
                .saturating_add(effective_page_size)
                .min(count);

            let offset_table_start = 6 * core::mem::size_of::<u32>();
            let mut out = Vec::new();
            for global_idx in page_start..page_end {
                let off_pos = offset_table_start + global_idx * core::mem::size_of::<u32>();
                if off_pos + 4 > bytes.len() {
                    break;
                }

                let cand_off = u32::from_le_bytes([
                    bytes[off_pos],
                    bytes[off_pos + 1],
                    bytes[off_pos + 2],
                    bytes[off_pos + 3],
                ]) as usize;

                let text = if use_ansi {
                    Self::read_ansi_z_at(&bytes, cand_off)
                } else {
                    Self::read_utf16_z_at(&bytes, cand_off)
                };
                if text.is_empty() {
                    continue;
                }

                let local_idx = out.len() as u32;
                out.push(BackendCandidate {
                    index: local_idx,
                    text,
                    comment: format!("imm{}#{list_index}", if use_ansi { "A" } else { "W" }),
                    quality: (100.0 - local_idx as f64).max(1.0),
                });
            }

            if !out.is_empty() || count > 0 {
                let selected_global = header.dwSelection as usize;
                let selected_index = if selected_global >= page_start && selected_global < page_end {
                    (selected_global - page_start) as u32
                } else {
                    0
                };

                return Ok(ImmCandidatePage {
                    candidates: out,
                    selected_index,
                    page_size: effective_page_size as u32,
                });
            }
        }

        Ok(ImmCandidatePage {
            candidates: Vec::new(),
            selected_index: 0,
            page_size: 0,
        })
    }

        fn refresh_from_imm(&mut self, max_candidates: usize) -> Result<(u32, u32), String> {
        let (himc, borrowed_ctx) = self.acquire_active_himc()?;

        let refresh_result = (|| {
            let comp = Self::read_imm_string(himc, GCS_COMPSTR)?;
            let read = Self::read_imm_string(himc, GCS_COMPREADSTR).unwrap_or_default();
            let page = Self::read_imm_candidates(himc, max_candidates)?;

            if !comp.is_empty() {
                self.composition = comp;
            }

            self.reading = if read.is_empty() {
                self.composition.clone()
            } else {
                read
            };

            self.candidates = page.candidates;
            self.timeline_log("S6_IMM_SNAPSHOT", || {
                format!(
                    "comp_len={} read_len={} cand_n={} selected={} page_size={}",
                    self.composition.chars().count(),
                    self.reading.chars().count(),
                    self.candidates.len(),
                    page.selected_index,
                    page.page_size
                )
            });
            Ok((page.selected_index, page.page_size))
        })();

        self.release_active_himc(himc, borrowed_ctx);

        refresh_result
    }

        fn apply_key_to_buffer(&mut self, key_event: &BackendKeyEvent) {
        if !key_event.key_down {
            return;
        }

        let mut virtual_key = key_event.virtual_key;
        if virtual_key == 0 && key_event.source_keycode <= 0xFF {
            virtual_key = key_event.source_keycode;
        }

        match virtual_key {
            0x08 => {
                self.composition.pop();
            }
            0x20..=0x7E => {
                let ch = (virtual_key as u8 as char).to_ascii_lowercase();
                if ch.is_ascii_alphanumeric() || ch == '\'' {
                    self.composition.push(ch);
                }
            }
            _ => {}
        }
    }

    fn not_ready_message(&self) -> String {
        self.init_error
            .clone()
            .unwrap_or_else(Self::unsupported_message)
    }

        fn ensure_runtime_ready(&self) -> Result<&WinImmRuntime, String> {
        self.runtime
            .as_ref()
            .ok_or_else(|| self.not_ready_message())
    }

        fn imm_runtime_available(&self) -> bool {
        match self.ensure_runtime_ready() {
            Ok(runtime) => runtime.ensure_context_alive().is_ok(),
            Err(_) => false,
        }
    }

        fn unsupported_message() -> String {
        "win_imm backend skeleton is not implemented yet (expected to be implemented with a minimal unsafe FFI boundary in this module)".to_string()
    }

    #[cfg(not(windows))]
    fn unsupported_message() -> String {
        "win_imm backend requires Windows runtime; current build is non-windows".to_string()
    }
}

impl ImeBackend for WinImmBackend {
    fn name(&self) -> &'static str {
        "win_imm"
    }

    fn snapshot(&self) -> BackendSnapshot {
        BackendSnapshot {
            composition: self.composition.clone(),
            backend_state_version: self.backend_state_version,
        }
    }

    fn reset_for_new_session(&mut self) -> u64 {
        self.backend_state_version += 1;
        self.clear_local_state();

                {
            if let Some(runtime) = self.runtime.as_ref() {
                let _ = runtime.ensure_context_alive();
            }
            self.timeline_log("S0_RESET_SESSION", || {
                let has_runtime = self.runtime.is_some();
                format!(
                    "backend_state_version={} has_runtime={} force_real_imm={}",
                    self.backend_state_version,
                    has_runtime,
                    self.force_real_imm
                )
            });
            let _ = self.activate_qq_ime();
        }

        self.backend_state_version
    }

    fn apply_key_event(
        &mut self,
        key_event: &BackendKeyEvent,
        _max_candidates: usize,
    ) -> Result<BackendEventResult, String> {
        self.backend_state_version += 1;

        #[cfg(not(windows))]
        let _ = key_event;

                {
            let max = if _max_candidates == 0 { 9 } else { _max_candidates };

            let _ = self.drive_qq_ime_key(key_event);
            self.apply_key_to_buffer(key_event);
            let (selected_index, page_size) = if self.imm_runtime_available() {
                match self.refresh_from_imm(max) {
                    Ok((selected, size)) => {
                        if self.candidates.is_empty() && !self.composition.is_empty() {
                            if self.force_real_imm {
                                (selected, if size > 0 { size } else { max as u32 })
                            } else {
                                self.refresh_fallback_candidates(max);
                                (0, max as u32)
                            }
                        } else {
                            (selected, if size > 0 { size } else { max as u32 })
                        }
                    }
                    Err(err) => {
                        if self.force_real_imm {
                            return Err(format!("IMM_REAL_REQUIRED: {err}"));
                        }
                        self.refresh_fallback_candidates(max);
                        (0, max as u32)
                    }
                }
            } else {
                if self.force_real_imm {
                    return Err("IMM_REAL_REQUIRED: runtime context unavailable".to_string());
                }
                self.refresh_fallback_candidates(max);
                (0, max as u32)
            };

            self.timeline_log("S7_EVENT_RESULT", || {
                format!(
                    "key_down={} comp_len={} read_len={} cand_n={} selected={} page_size={} state_version={}",
                    key_event.key_down,
                    self.composition.chars().count(),
                    self.reading.chars().count(),
                    self.candidates.len(),
                    selected_index,
                    page_size,
                    self.backend_state_version
                )
            });

            Ok(BackendEventResult {
                composition: self.composition.clone(),
                reading: self.reading.clone(),
                candidates: self.candidates.clone(),
                selected_index,
                page_size,
                backend_state_version: self.backend_state_version,
            })
        }

        #[cfg(not(windows))]
        {
            Err(self.not_ready_message())
        }
    }

    fn query_candidates(
        &mut self,
        _input_snapshot: &str,
        _max_candidates: usize,
    ) -> Result<BackendQueryResult, String> {
        self.backend_state_version += 1;

                {
            if !_input_snapshot.is_empty() {
                self.composition = _input_snapshot.to_string();
            }

            let max = if _max_candidates == 0 { 9 } else { _max_candidates };
            let (selected_index, page_size) = if self.imm_runtime_available() {
                match self.refresh_from_imm(max) {
                    Ok((selected, size)) => {
                        if self.candidates.is_empty() && !self.composition.is_empty() {
                            if self.force_real_imm {
                                return Err(
                                    "IMM_REAL_REQUIRED: no candidate list returned from IMM"
                                        .to_string(),
                                );
                            }
                            self.refresh_fallback_candidates(max);
                            (0, max as u32)
                        } else {
                            (selected, if size > 0 { size } else { max as u32 })
                        }
                    }
                    Err(err) => {
                        if self.force_real_imm {
                            return Err(format!("IMM_REAL_REQUIRED: {err}"));
                        }
                        self.refresh_fallback_candidates(max);
                        (0, max as u32)
                    }
                }
            } else {
                if self.force_real_imm {
                    return Err("IMM_REAL_REQUIRED: runtime context unavailable".to_string());
                }
                self.refresh_fallback_candidates(max);
                (0, max as u32)
            };

            self.timeline_log("S8_QUERY_RESULT", || {
                format!(
                    "input_len={} comp_len={} read_len={} cand_n={} selected={} page_size={} state_version={}",
                    _input_snapshot.chars().count(),
                    self.composition.chars().count(),
                    self.reading.chars().count(),
                    self.candidates.len(),
                    selected_index,
                    page_size,
                    self.backend_state_version
                )
            });

            Ok(BackendQueryResult {
                composition: self.composition.clone(),
                reading: self.reading.clone(),
                candidates: self.candidates.clone(),
                selected_index,
                page_size,
                backend_state_version: self.backend_state_version,
            })
        }

        #[cfg(not(windows))]
        {
            Err(self.not_ready_message())
        }
    }

    fn commit_selection(
        &mut self,
        _committed_text: &str,
        _candidate_index: usize,
    ) -> Result<BackendCommitResult, String> {
        self.backend_state_version += 1;

                {
            let mut committed = _committed_text.to_string();
            if committed.is_empty() {
                if let Some(item) = self.candidates.get(_candidate_index) {
                    committed = item.text.clone();
                }
            }

            if committed.is_empty() {
                return Err("no committed_text and candidate_index is invalid".to_string());
            }

            self.clear_local_state();
            Ok(BackendCommitResult {
                committed_text: committed,
                backend_state_version: self.backend_state_version,
            })
        }

        #[cfg(not(windows))]
        {
            Err(self.not_ready_message())
        }
    }

    fn reset(&mut self) -> Result<u64, String> {
        Ok(self.reset_for_new_session())
    }

    fn set_debug_timeline_enabled(&mut self, enabled: bool) {
                {
            self.trace_timeline = enabled;
        }

        #[cfg(not(windows))]
        {
            let _ = enabled;
        }
    }

    fn drain_debug_timeline(&mut self) -> Vec<String> {
        std::mem::take(self.debug_timeline.get_mut())
    }
}
