pub mod channel_adapter;
pub mod imm_ops;
pub mod keys;
pub mod session;
pub mod thread_pump;
pub mod vk_map;

use crate::backend::RimeBackend;
use crate::proto::rime_service_v2::{KeyEvent, RimeContextProto};
use crate::win_imm::imm_ops::ImeFunctions;
use std::collections::HashMap;
use std::sync::OnceLock;
use windows::Win32::Foundation::{BOOL, LPARAM, WPARAM};
use windows::Win32::UI::Input::Ime::{
    CPS_COMPLETE, GCS_RESULTSTR, IMN_SETCONVERSIONMODE, IMN_SETOPENSTATUS, ISC_SHOWUIALL,
    ImmNotifyIME, NI_COMPOSITIONSTR, NI_SELECTCANDIDATESTR, NOTIFY_IME_INDEX,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, SetKeyboardState, ToUnicode, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL,
    VK_LMENU, VK_LSHIFT, VK_MENU, VK_SHIFT, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetWindowThreadProcessId, PeekMessageW, SendMessageW, TranslateMessage, MSG,
    PM_REMOVE, WM_CHAR, WM_IME_CHAR, WM_IME_COMPOSITION, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

const IPHK_PROCESSBYIME: u32 = 0x00000002;
const MAX_PUMP_MESSAGES: usize = 256;

struct PumpResult {
    result_text: String,
    char_text: String,
    saw_result_message: bool,
}

/// The Win32 adapter that implements the RimeBackend trait.
pub struct ImmRimeAdapter {
    ime_functions: Option<ImeFunctions>,
    sessions: HashMap<usize, session::WinImmSession>,
    show_window: bool,
}

static IME_FUNCS: OnceLock<Option<ImeFunctions>> = OnceLock::new();

impl Default for ImmRimeAdapter {
    fn default() -> Self {
        Self::new("C:\\windows\\system32\\QQPinyin.ime", false)
    }
}

impl ImmRimeAdapter {
    pub fn new(ime_path: &str, show_window: bool) -> Self {
        let ime_path_owned = ime_path.to_string();
        let ime_functions = *IME_FUNCS.get_or_init(move || {
            use windows::core::{HSTRING, PCWSTR};
            let hstring_path = HSTRING::from(ime_path_owned.as_str());
            match crate::win_imm::imm_ops::load_ime_dll(PCWSTR::from_raw(hstring_path.as_ptr())) {
                Ok(funcs) => {
                    unsafe {
                        let mut ime_info = std::mem::zeroed();
                        let mut class_name = [0u16; 256];
                        let _ = (funcs.inquire)(&mut ime_info, class_name.as_mut_ptr(), 0);
                    }
                    Some(funcs)
                }
                Err(e) => {
                    println!("Failed to load IME from {}: {:?}", ime_path_owned, e);
                    None
                }
            }
        });

        if show_window {
            tracing::info!("show_window flag is enabled for IME sessions");
        }

        Self {
            ime_functions,
            sessions: HashMap::new(),
            show_window,
        }
    }

    fn append_commit(session: &mut session::WinImmSession, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut s = session.pending_commit.take().unwrap_or_default();
        s.push_str(text);
        session.pending_commit = Some(s);
    }

    fn pump_messages_for_hwnd(
        session: &mut session::WinImmSession,
        filter_hwnd: Option<windows::Win32::Foundation::HWND>,
    ) -> PumpResult {
        let mut out = PumpResult {
            result_text: String::new(),
            char_text: String::new(),
            saw_result_message: false,
        };

        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            let mut count = 0usize;
            let hwnd_filter = filter_hwnd.unwrap_or_default();

            while count < MAX_PUMP_MESSAGES
                && PeekMessageW(&mut msg, hwnd_filter, 0, 0, PM_REMOVE).as_bool()
            {
                count += 1;
                tracing::debug!(
                    "pump_messages: hwnd=0x{:X} msg=0x{:X} wp=0x{:X} lp=0x{:X} filter={}",
                    msg.hwnd.0 as usize,
                    msg.message,
                    msg.wParam.0,
                    msg.lParam.0 as usize,
                    if filter_hwnd.is_some() { "window" } else { "thread" }
                );

                if msg.message == WM_IME_COMPOSITION
                    && (msg.lParam.0 as u32 & GCS_RESULTSTR.0 as u32) != 0
                {
                    out.saw_result_message = true;
                    if let Some(rstr) = crate::win_imm::imm_ops::get_result_string(session.himc) {
                        out.result_text.push_str(&rstr);
                    }
                } else if msg.message == WM_IME_CHAR || msg.message == WM_CHAR {
                    if let Some(ch) = std::char::from_u32(msg.wParam.0 as u32) {
                        out.char_text.push(ch);
                    }
                }

                let _ = TranslateMessage(&msg);
                let _ = DispatchMessageW(&msg);
            }
        }

        out
    }

    fn pump_window_messages(session: &mut session::WinImmSession) -> PumpResult {
        Self::pump_messages_for_hwnd(session, Some(session.target_hwnd))
    }

    fn pump_thread_messages(session: &mut session::WinImmSession) -> PumpResult {
        Self::pump_messages_for_hwnd(session, None)
    }

    fn harvest_thread_messages(session: &mut session::WinImmSession, reason: &str) {
        let pumped = Self::pump_thread_messages(session);
        let target_text = crate::win_imm::imm_ops::get_window_text(session.target_hwnd);
        if !pumped.result_text.is_empty() {
            tracing::info!(
                "harvest_thread_messages({reason}): commit={:?} target_text={:?}",
                pumped.result_text,
                target_text
            );
            Self::append_commit(session, &pumped.result_text);
        } else if !pumped.char_text.is_empty() {
            tracing::info!(
                "harvest_thread_messages({reason}): commit(char)={:?} target_text={:?}",
                pumped.char_text,
                target_text
            );
            Self::append_commit(session, &pumped.char_text);
        } else if pumped.saw_result_message {
            tracing::warn!(
                "harvest_thread_messages({reason}): saw GCS_RESULTSTR message but result text was empty; target_text={:?}",
                target_text
            );
        } else if !target_text.is_empty() {
            tracing::info!(
                "harvest_thread_messages({reason}): no message-derived commit; target_text={:?}",
                target_text
            );
        }
    }

    fn activate_ime_context(session: &mut session::WinImmSession, ime: &ImeFunctions) {
        unsafe {
            let selected = (ime.select)(session.himc, BOOL(1)).as_bool();
            let activated = ime
                .set_active_context
                .map(|set_active| set_active(session.himc, BOOL(1)).as_bool())
                .unwrap_or(false);
            let open = crate::win_imm::imm_ops::imm_set_open_status(session.himc, true);
            let native_mode =
                crate::win_imm::imm_ops::imm_set_native_conversion_status(session.himc);
            tracing::info!(
                "activate_ime_context: hwnd=0x{:X} target=0x{:X} himc=0x{:X} select={} active={} open={} native_mode={}",
                session.hwnd.0 as usize,
                session.target_hwnd.0 as usize,
                session.himc.0 as usize,
                selected,
                activated,
                open,
                native_mode
            );

            let _ = SendMessageW(
                session.target_hwnd,
                windows::Win32::UI::WindowsAndMessaging::WM_IME_SETCONTEXT,
                WPARAM(1),
                LPARAM(ISC_SHOWUIALL as isize),
            );

            let def_ime_wnd =
                crate::win_imm::imm_ops::imm_get_default_ime_wnd(session.target_hwnd);
            tracing::info!(
                "activate_ime_context: default_ime_wnd=0x{:X}",
                def_ime_wnd.0 as usize
            );
            if def_ime_wnd.0 != std::ptr::null_mut() && def_ime_wnd != session.target_hwnd {
                let _ = SendMessageW(
                    def_ime_wnd,
                    windows::Win32::UI::WindowsAndMessaging::WM_IME_SETCONTEXT,
                    WPARAM(1),
                    LPARAM(ISC_SHOWUIALL as isize),
                );
                let _ = SendMessageW(
                    def_ime_wnd,
                    windows::Win32::UI::WindowsAndMessaging::WM_IME_SELECT,
                    WPARAM(1),
                    LPARAM(session.himc.0 as isize),
                );
                let _ = SendMessageW(
                    def_ime_wnd,
                    windows::Win32::UI::WindowsAndMessaging::WM_IME_NOTIFY,
                    WPARAM(IMN_SETOPENSTATUS as usize),
                    LPARAM(0),
                );
                let _ = SendMessageW(
                    def_ime_wnd,
                    windows::Win32::UI::WindowsAndMessaging::WM_IME_NOTIFY,
                    WPARAM(IMN_SETCONVERSIONMODE as usize),
                    LPARAM(0),
                );
            }
        }

        let _ = Self::pump_thread_messages(session);
    }

    fn dispatch_ime_transmsgs(
        session: &mut session::WinImmSession,
        ime: &ImeFunctions,
        ime_uvirt: u32,
        key_state: &[u8; 256],
    ) -> PumpResult {
        use windows::Win32::UI::Input::Ime::{TRANSMSG, TRANSMSGLIST};

        const MAX_TRANS_MSGS: usize = 256;
        let header_size = std::mem::offset_of!(TRANSMSGLIST, TransMsg);
        let transmsg_size = std::mem::size_of::<TRANSMSG>();
        let total_bytes = header_size + transmsg_size * MAX_TRANS_MSGS;

        let mut out = PumpResult {
            result_text: String::new(),
            char_text: String::new(),
            saw_result_message: false,
        };

        let mut trans_buf = vec![0u64; (total_bytes + 7) / 8];
        let list_ptr = trans_buf.as_mut_ptr() as *mut TRANSMSGLIST;
        unsafe {
            (*list_ptr).uMsgCount = MAX_TRANS_MSGS as u32;
        }

        let msg_count = unsafe {
            (ime.to_ascii_ex)(ime_uvirt, 0, key_state.as_ptr(), list_ptr, 0, session.himc)
        } as i32;
        tracing::debug!("ImeToAsciiEx => {}", msg_count);

        let actual_count = if msg_count > 0 {
            std::cmp::min(msg_count as usize, MAX_TRANS_MSGS)
        } else {
            0
        };
        let msgs_ptr = unsafe { (*list_ptr).TransMsg.as_ptr() };

        for i in 0..actual_count {
            let msg = unsafe { &*msgs_ptr.add(i) };
            if msg.message == 0 {
                continue;
            }
            tracing::debug!(
                "TRANSMSG[{}]: msg=0x{:X} wp=0x{:X} lp=0x{:X}",
                i,
                msg.message,
                msg.wParam.0 as u32,
                msg.lParam.0 as u32
            );
            if msg.message == WM_IME_COMPOSITION && (msg.lParam.0 as u32 & GCS_RESULTSTR.0 as u32) != 0 {
                out.saw_result_message = true;
                if out.result_text.is_empty() {
                    if let Some(rstr) = crate::win_imm::imm_ops::get_result_string(session.himc) {
                        out.result_text.push_str(&rstr);
                    }
                }
            }
            if msg.message == WM_IME_CHAR || msg.message == WM_CHAR {
                if let Some(ch) = std::char::from_u32(msg.wParam.0 as u32) {
                    out.char_text.push(ch);
                }
            }
            unsafe {
                let _ = SendMessageW(session.target_hwnd, msg.message, msg.wParam, msg.lParam);
            }
        }

        let pumped = Self::pump_window_messages(session);
        if out.result_text.is_empty() {
            out.result_text.push_str(&pumped.result_text);
        }
        if out.result_text.is_empty() {
            out.char_text.push_str(&pumped.char_text);
        }
        out.saw_result_message |= pumped.saw_result_message;

        // Always check GCS_RESULTSTR after ImeToAsciiEx, even without a visible
        // WM_IME_COMPOSITION message. The IME may write the result string to the
        // HIMC and send the composition message internally via ImmGenerateMessage,
        // which uses synchronous SendMessageW — the message is dispatched and
        // consumed by DefWindowProcW before our pump can observe it.
        if out.result_text.is_empty() {
            if let Some(rstr) = crate::win_imm::imm_ops::get_result_string(session.himc) {
                tracing::info!(
                    "dispatch_ime_transmsgs: GCS_RESULTSTR available without visible message: {:?}",
                    rstr
                );
                out.result_text.push_str(&rstr);
            }
        }

        out
    }

    pub async fn process_vk(
        &mut self,
        session_id: usize,
        vk: VIRTUAL_KEY,
        modifiers: u32,
        is_keyup: bool,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return false;
        };

        let Some(ime) = self.ime_functions.as_ref() else {
            return false;
        };

        let mut key_state = [0u8; 256];
        let is_shift = (modifiers & 1) != 0;
        let is_ctrl = (modifiers & 2) != 0;
        let is_alt = (modifiers & 4) != 0;

        if is_shift {
            key_state[VK_SHIFT.0 as usize] = 0x80;
            key_state[VK_LSHIFT.0 as usize] = 0x80;
        }
        if is_ctrl {
            key_state[VK_CONTROL.0 as usize] = 0x80;
            key_state[VK_LCONTROL.0 as usize] = 0x80;
        }
        if is_alt {
            key_state[VK_MENU.0 as usize] = 0x80;
            key_state[VK_LMENU.0 as usize] = 0x80;
        }

        let vk_u32 = vk.0 as u32;
        let l_key_data = crate::win_imm::vk_map::make_l_key_data(vk, is_keyup, is_alt);
        let scan_code = (l_key_data >> 16) & 0xFF;
        let key_message = match (is_keyup, is_alt) {
            (true, true) => WM_SYSKEYUP,
            (false, true) => WM_SYSKEYDOWN,
            (true, false) => WM_KEYUP,
            (false, false) => WM_KEYDOWN,
        };

        tracing::debug!(
            "process_vk: vk=0x{:X} modifiers={} keyup={} message=0x{:X}",
            vk_u32,
            modifiers,
            is_keyup,
            key_message
        );

        unsafe {
            let _ = SetKeyboardState(&key_state);
        }

        let mut unicode_buf = [0u16; 2];
        let unicode_len =
            unsafe { ToUnicode(vk_u32, scan_code, Some(&key_state), &mut unicode_buf, 0) };
        // NT's FE IME path consumes HIWORD(uVirtKey) as the translated character
        // and LOBYTE(uVirtKey) as the physical virtual-key code.
        let ime_uvirt = if unicode_len > 0 {
            vk_u32 | ((unicode_buf[0] as u32) << 16)
        } else {
            vk_u32
        };

        unsafe {
            let _ = SendMessageW(
                session.target_hwnd,
                key_message,
                WPARAM(vk_u32 as usize),
                LPARAM(l_key_data as isize),
            );
        }

        let mut process_id = 0u32;
        let thread_id =
            unsafe {
                GetWindowThreadProcessId(session.target_hwnd, Some(&mut process_id as *mut u32))
            };
        let hkl = unsafe { GetKeyboardLayout(thread_id) };
        let imm_flags =
            crate::win_imm::imm_ops::imm_process_key(session.target_hwnd, hkl, vk_u32, l_key_data);

        tracing::debug!("ImmProcessKey => 0x{:X}", imm_flags);

        let accepted = (imm_flags & IPHK_PROCESSBYIME) != 0;
        if accepted {
            let translated = crate::win_imm::imm_ops::imm_translate_message(
                session.target_hwnd,
                key_message,
                vk_u32 as usize,
                l_key_data,
            );
            tracing::debug!("ImmTranslateMessage => {}", translated);
        }

        let mut pumped = Self::pump_thread_messages(session);
        if !pumped.result_text.is_empty() {
            tracing::info!("commit: '{}'", pumped.result_text);
            Self::append_commit(session, &pumped.result_text);
        } else if !pumped.char_text.is_empty() {
            tracing::info!("commit(char): '{}'", pumped.char_text);
            Self::append_commit(session, &pumped.char_text);
        } else if pumped.saw_result_message {
            tracing::warn!(
                "WM_IME_COMPOSITION(GCS_RESULTSTR) observed but ImmGetCompositionStringW returned empty"
            );
        }

        let direct_consumed =
            unsafe { (ime.process_key)(session.himc, vk_u32, l_key_data, key_state.as_ptr()).as_bool() };
        tracing::debug!("ImeProcessKey => {}", direct_consumed);

        if direct_consumed {
            let direct = Self::dispatch_ime_transmsgs(session, ime, ime_uvirt, &key_state);
            if !direct.result_text.is_empty() {
                tracing::info!("commit: '{}'", direct.result_text);
                Self::append_commit(session, &direct.result_text);
            } else if !direct.char_text.is_empty() {
                tracing::info!("commit(char): '{}'", direct.char_text);
                Self::append_commit(session, &direct.char_text);
            } else if direct.saw_result_message {
                tracing::warn!(
                    "ImeToAsciiEx signaled GCS_RESULTSTR but ImmGetCompositionStringW returned empty"
                );
            }
            pumped = Self::pump_thread_messages(session);
            if !pumped.result_text.is_empty() {
                tracing::info!("commit(post-direct): '{}'", pumped.result_text);
                Self::append_commit(session, &pumped.result_text);
            } else if !pumped.char_text.is_empty() {
                tracing::info!("commit(post-direct-char): '{}'", pumped.char_text);
                Self::append_commit(session, &pumped.char_text);
            }
        }

        if !is_keyup {
            let keyup_lparam = crate::win_imm::vk_map::make_l_key_data(vk, true, is_alt);
            let keyup_message = if is_alt { WM_SYSKEYUP } else { WM_KEYUP };
            unsafe {
                let _ = SendMessageW(
                    session.target_hwnd,
                    keyup_message,
                    WPARAM(vk_u32 as usize),
                    LPARAM(keyup_lparam as isize),
                );
            }
            let keyup_pumped = Self::pump_thread_messages(session);
            if !keyup_pumped.result_text.is_empty() {
                tracing::info!("commit(keyup): '{}'", keyup_pumped.result_text);
                Self::append_commit(session, &keyup_pumped.result_text);
            } else if !keyup_pumped.char_text.is_empty() {
                tracing::info!("commit(keyup-char): '{}'", keyup_pumped.char_text);
                Self::append_commit(session, &keyup_pumped.char_text);
            }
        }

        // Final GCS_RESULTSTR check: if the IME produced a result string but
        // we never observed it through message-based paths (e.g. the IME
        // used ImmGenerateMessage internally, which dispatches synchronously
        // through DefWindowProcW before our pump can observe it), read it
        // directly from the HIMC.
        if session.pending_commit.is_none() {
            if let Some(rstr) = crate::win_imm::imm_ops::get_result_string(session.himc) {
                tracing::info!("process_vk: final GCS_RESULTSTR check found: {:?}", rstr);
                Self::append_commit(session, &rstr);
            }
        }

        accepted || direct_consumed
    }
}

impl Drop for ImmRimeAdapter {
    fn drop(&mut self) {
        for (_, session) in self.sessions.drain() {
            if let Some(ime) = &self.ime_functions {
                unsafe {
                    let _ = (ime.select)(session.himc, BOOL(0));
                }
            }
            session.destroy();
        }
    }
}

#[tonic::async_trait]
impl RimeBackend for ImmRimeAdapter {
    async fn open_session(&mut self) -> Option<usize> {
        let id = self.sessions.len() + 1;
        let ime = self.ime_functions?;
        match session::WinImmSession::create(id, ime.h_module, self.show_window) {
            Ok(mut session) => {
                Self::activate_ime_context(&mut session, &ime);
                self.sessions.insert(id, session);
                Some(id)
            }
            Err(_) => None,
        }
    }

    async fn destroy_session(&mut self, session_id: usize) {
        if let Some(session) = self.sessions.remove(&session_id) {
            if let Some(ime) = &self.ime_functions {
                unsafe {
                    let _ = (ime.select)(session.himc, BOOL(0));
                }
            }
            session.destroy();
        }
    }

    async fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool {
        let rime_mod = key.modifier;
        let is_keyup = (rime_mod & (1 << 30)) != 0;

        let is_alt = (rime_mod & 8) != 0;
        let is_ctrl = (rime_mod & 4) != 0;
        if (is_ctrl || is_alt) && !(is_ctrl && is_alt) {
            return false;
        }

        let is_shift =
            (rime_mod & 1) != 0 || crate::win_imm::vk_map::is_shifted_char(key.keycode);

        let mut win_mod = 0u32;
        if is_shift {
            win_mod |= 1;
        }
        if is_ctrl {
            win_mod |= 2;
        }
        if is_alt {
            win_mod |= 4;
        }

        self.process_vk(
            session_id,
            crate::win_imm::vk_map::rime_to_vk(key.keycode),
            win_mod,
            is_keyup,
        )
        .await
    }

    async fn get_context(&mut self, session_id: usize) -> RimeContextProto {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            Self::harvest_thread_messages(session, "get_context");
            let comp_str = crate::win_imm::imm_ops::get_composition_string(session.himc);
            let menu_proto = crate::win_imm::imm_ops::get_candidate_list(session.himc);

            let mut context = RimeContextProto {
                composition: None,
                menu: menu_proto,
                commit_text_preview: String::new(),
            };

            if let Some(comp_data) = comp_str {
                context.composition = Some(crate::proto::rime_service_v2::CompositionProto {
                    length: comp_data.text.len() as i32,
                    cursor_pos: comp_data.cursor_pos,
                    sel_start: comp_data.sel_start,
                    sel_end: comp_data.sel_end,
                    preedit: comp_data.text,
                });
            }

            return context;
        }

        RimeContextProto {
            composition: None,
            menu: None,
            commit_text_preview: String::new(),
        }
    }

    async fn get_commit(&mut self, session_id: usize) -> Option<String> {
        self.sessions.get_mut(&session_id).and_then(|session| {
            Self::harvest_thread_messages(session, "get_commit");
            session.pending_commit.take()
        })
    }

    async fn select_candidate(&mut self, session_id: usize, index: usize) -> bool {
        let Some(ime) = self.ime_functions else {
            return false;
        };
        let Some(session) = self.sessions.get(&session_id) else {
            return false;
        };

        let before_comp = crate::win_imm::imm_ops::get_composition_string(session.himc)
            .map(|comp| comp.text)
            .unwrap_or_default();
        let before_menu = crate::win_imm::imm_ops::get_candidate_list(session.himc)
            .map(|menu| {
                menu.candidates
                    .into_iter()
                    .map(|cand| cand.text)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        tracing::info!(
            "select_candidate: session={} index={} before_comp={:?} before_candidates={:?}",
            session_id,
            index,
            before_comp,
            before_menu
        );

        let selection_vk = match index {
            0..=8 => Some((b'1' + index as u8) as u32),
            9 => Some(b'0' as u32),
            _ => None,
        };

        if let Some(selection_vk) = selection_vk {
            tracing::info!(
                "select_candidate: trying numeric selection key first, index={} vk=0x{:X}",
                index,
                selection_vk
            );
            let selected_by_key = self
                .process_vk(session_id, VIRTUAL_KEY(selection_vk as u16), 0, false)
                .await;
            tracing::info!(
                "select_candidate: process_vk(selection) => {}",
                selected_by_key
            );

            // Read post-selection snapshot into locals, then release the borrow
            // before potentially calling process_vk again for Return confirmation.
            let (keyed_comp, keyed_result, keyed_menu, has_pending) = {
                if let Some(session) = self.sessions.get(&session_id) {
                    let kc = crate::win_imm::imm_ops::get_composition_string(session.himc)
                        .map(|comp| comp.text)
                        .unwrap_or_default();
                    let kr = crate::win_imm::imm_ops::get_result_string(session.himc)
                        .unwrap_or_default();
                    let km = crate::win_imm::imm_ops::get_candidate_list(session.himc)
                        .map(|menu| {
                            menu.candidates
                                .into_iter()
                                .map(|cand| cand.text)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let hp = session.pending_commit.is_some();
                    tracing::info!(
                        "select_candidate: after numeric key comp={:?} result={:?} candidates={:?} pending_commit={}",
                        kc, kr, km, hp
                    );
                    (kc, kr, km, hp)
                } else {
                    (String::new(), String::new(), Vec::new(), false)
                }
            };

            if has_pending
                || keyed_comp != before_comp
                || keyed_menu != before_menu
                || !keyed_result.is_empty()
                || selected_by_key
            {
                // The IME consumed the numeric selection key. When composition survives
                // (e.g. QQ Pinyin two-step select+confirm), send Return to finalize.
                // When composition is already cleared, the IME consumed the result
                // internally (e.g. Sogou64); return true but no commit to harvest.
                if !keyed_comp.is_empty() && !has_pending {
                    tracing::info!(
                        "select_candidate: composition still present after numeric key, sending Return to confirm"
                    );
                    self.process_vk(session_id, VK_RETURN, 0, false).await;
                    if let Some(session) = self.sessions.get_mut(&session_id) {
                        Self::harvest_thread_messages(session, "select_candidate_return_confirm");
                    }
                }
                return true;
            }
        }

        let Some(session) = self.sessions.get_mut(&session_id) else {
            return false;
        };

        unsafe {
            let notified = ime
                .notify_ime
                .map(|notify_ime| {
                    notify_ime(
                        session.himc,
                        NI_SELECTCANDIDATESTR.0,
                        0,
                        index as u32,
                    )
                })
                .unwrap_or_else(|| {
                    ImmNotifyIME(
                        session.himc,
                        NI_SELECTCANDIDATESTR,
                        NOTIFY_IME_INDEX(0),
                        index as u32,
                    )
                });
            tracing::info!(
                "select_candidate: NotifyIME(NI_SELECTCANDIDATESTR, index={}) => {}",
                index,
                notified.as_bool()
            );
        }

        let mut pumped = Self::pump_thread_messages(session);
        tracing::info!(
            "select_candidate: pumped result_text={:?} char_text={:?} saw_result_message={}",
            pumped.result_text,
            pumped.char_text,
            pumped.saw_result_message
        );
        if !pumped.result_text.is_empty() {
            Self::append_commit(session, &pumped.result_text);
        } else if !pumped.char_text.is_empty() {
            Self::append_commit(session, &pumped.char_text);
        }
        let after_comp = crate::win_imm::imm_ops::get_composition_string(session.himc)
            .map(|comp| comp.text)
            .unwrap_or_default();
        let after_result =
            crate::win_imm::imm_ops::get_result_string(session.himc).unwrap_or_default();
        let after_target_text = crate::win_imm::imm_ops::get_window_text(session.target_hwnd);
        let after_menu = crate::win_imm::imm_ops::get_candidate_list(session.himc)
            .map(|menu| {
                menu.candidates
                    .into_iter()
                    .map(|cand| cand.text)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        tracing::info!(
            "select_candidate: after_comp={:?} after_result={:?} after_target_text={:?} after_candidates={:?} pending_commit={:?}",
            after_comp,
            after_result,
            after_target_text,
            after_menu,
            session.pending_commit
        );

        if session.pending_commit.is_none()
            && after_comp == before_comp
            && after_menu == before_menu
            && !before_comp.is_empty()
        {
            tracing::info!(
                "select_candidate: selection produced no visible state change; trying NotifyIME(NI_COMPOSITIONSTR, CPS_COMPLETE)"
            );
            unsafe {
                let committed = ime
                    .notify_ime
                    .map(|notify_ime| {
                        notify_ime(session.himc, NI_COMPOSITIONSTR.0, CPS_COMPLETE.0, 0)
                    })
                    .unwrap_or_else(|| ImmNotifyIME(session.himc, NI_COMPOSITIONSTR, CPS_COMPLETE, 0));
                tracing::info!(
                    "select_candidate: NotifyIME(NI_COMPOSITIONSTR, CPS_COMPLETE) => {}",
                    committed.as_bool()
                );
            }

            let generated = crate::win_imm::imm_ops::imm_generate_message(session.himc);
            tracing::info!("select_candidate: ImmGenerateMessage(himc) => {}", generated);

            pumped = Self::pump_thread_messages(session);
            tracing::info!(
                "select_candidate: commit pump result_text={:?} char_text={:?} saw_result_message={}",
                pumped.result_text,
                pumped.char_text,
                pumped.saw_result_message
            );
            if !pumped.result_text.is_empty() {
                Self::append_commit(session, &pumped.result_text);
            } else if !pumped.char_text.is_empty() {
                Self::append_commit(session, &pumped.char_text);
            }

            let final_comp = crate::win_imm::imm_ops::get_composition_string(session.himc)
                .map(|comp| comp.text)
                .unwrap_or_default();
            let final_result =
                crate::win_imm::imm_ops::get_result_string(session.himc).unwrap_or_default();
            let final_target_text = crate::win_imm::imm_ops::get_window_text(session.target_hwnd);
            let final_menu = crate::win_imm::imm_ops::get_candidate_list(session.himc)
                .map(|menu| {
                    menu.candidates
                        .into_iter()
                        .map(|cand| cand.text)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            tracing::info!(
                "select_candidate: final_comp={:?} final_result={:?} final_target_text={:?} final_candidates={:?} pending_commit={:?}",
                final_comp,
                final_result,
                final_target_text,
                final_menu,
                session.pending_commit
            );
        }
        true
    }
}
