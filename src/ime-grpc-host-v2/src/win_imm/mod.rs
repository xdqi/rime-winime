// src/win_imm/mod.rs
pub mod channel_adapter;
pub mod imm_ops;
pub mod keys;
pub mod punct_map;
pub mod session;
pub mod thread_pump;
pub mod vk_map;

use crate::backend::RimeBackend;
use crate::win_imm::imm_ops::ImeFunctions;
use std::collections::HashMap;
use std::sync::OnceLock;
use windows::Win32::UI::Input::Ime::GCS_RESULTSTR;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    PeekMessageW, MSG, PM_REMOVE, WM_CHAR, WM_IME_CHAR, WM_IME_COMPOSITION,
};

/// Known paired punctuation: maps opener to closer.
/// When the IME commits "opener+closer" as a single result string,
/// we split them: commit only the opener, store the closer to append to the next commit.
fn split_paired_punct(s: &str) -> Option<(&str, &str)> {
    // Each pair: (opener, closer) as &str
    const PAIRS: &[(&str, &str)] = &[
        ("\u{FF08}", "\u{FF09}"), // （ ）
        ("\u{201C}", "\u{201D}"), // " "
        ("\u{2018}", "\u{2019}"), // ' '
        ("\u{3010}", "\u{3011}"), // 【 】
        ("\u{300A}", "\u{300B}"), // 《 》
        ("\u{300C}", "\u{300D}"), // 「 」
        ("\u{300E}", "\u{300F}"), // 『 』
        ("\u{3008}", "\u{3009}"), // 〈 〉
    ];
    for &(opener, closer) in PAIRS {
        if s == format!("{}{}", opener, closer) {
            return Some((opener, closer));
        }
    }
    None
}
use crate::proto::rime_service_v2::{KeyEvent, RimeContextProto};

extern "system" {
    pub fn FreeLibrary(
        hLibModule: windows::Win32::Foundation::HMODULE,
    ) -> windows::Win32::Foundation::BOOL;
}

/// The Win32 adapter that implements the RimeBackend trait.
pub struct ImmRimeAdapter {
    ime_functions: Option<ImeFunctions>,
    sessions: HashMap<usize, session::WinImmSession>,
    show_window: bool,
    enable_punct_fallback: bool,
}

static IME_FUNCS: OnceLock<Option<ImeFunctions>> = OnceLock::new();

impl Default for ImmRimeAdapter {
    fn default() -> Self {
        Self::new("C:\\windows\\system32\\QQPinyin.ime", false, true)
    }
}

impl ImmRimeAdapter {
    pub fn new(ime_path: &str, show_window: bool, enable_punct_fallback: bool) -> Self {
        let ime_path_owned = ime_path.to_string();
        let ime_functions = *IME_FUNCS.get_or_init(move || {
            use windows::core::{HSTRING, PCWSTR};
            let hstring_path = HSTRING::from(ime_path_owned.as_str());
            match crate::win_imm::imm_ops::load_ime_dll(PCWSTR::from_raw(hstring_path.as_ptr())) {
                Ok(funcs) => {
                    // Global initialize
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
            enable_punct_fallback,
        }
    }

    pub async fn process_vk(
        &mut self,
        session_id: usize,
        vk: VIRTUAL_KEY,
        modifiers: u32,
        is_keyup: bool,
    ) -> bool {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            if let Some(ime) = &self.ime_functions {
                let mut key_state = [0u8; 256];

                // Win modifiers roughly: 1=Shift, 2=Ctrl, 4=Alt
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
                let l_key_data = crate::win_imm::vk_map::make_l_key_data(vk_u32, is_keyup, is_alt);

                tracing::debug!("process_vk: vk=0x{:X} modifiers={}", vk_u32, modifiers);

                let mut char_buf = [0u16; 2];
                let scan_code = (l_key_data >> 16) & 0xFF;
                let char_len =
                    unsafe { ToUnicode(vk_u32, scan_code, Some(&key_state), &mut char_buf, 0) };
                let ascii_code = if char_len > 0 {
                    char_buf[0] as u32
                } else {
                    vk_u32
                };

                let is_consumed = unsafe {
                    SetKeyboardState(&key_state).ok();
                    let res =
                        (ime.process_key)(session.himc, vk_u32, l_key_data, key_state.as_ptr());
                    res
                };

                tracing::debug!("ImeProcessKey => {:?}", is_consumed);

                if is_consumed.as_bool() {
                    use windows::Win32::UI::Input::Ime::{TRANSMSG, TRANSMSGLIST};

                    const MAX_TRANS_MSGS: usize = 256;
                    let header_size = std::mem::offset_of!(TRANSMSGLIST, TransMsg);
                    let transmsg_size = std::mem::size_of::<TRANSMSG>();
                    let total_bytes = header_size + transmsg_size * MAX_TRANS_MSGS;

                    let mut trans_buf = vec![0u64; (total_bytes + 7) / 8];
                    let list_ptr = trans_buf.as_mut_ptr() as *mut TRANSMSGLIST;
                    unsafe {
                        (*list_ptr).uMsgCount = MAX_TRANS_MSGS as u32;
                    }

                    let msg_count = unsafe {
                        (ime.to_ascii_ex)(
                            vk_u32,
                            scan_code,
                            key_state.as_ptr(),
                            list_ptr,
                            0,
                            session.himc,
                        )
                    } as i32;
                    tracing::debug!("ToAsciiEx => {}", msg_count);

                    // Phase 1: iterate TRANSMSGLIST for commit signals
                    let mut has_commit_msg = false;
                    let mut char_commits = String::new();
                    let actual_count = if msg_count > 0 {
                        std::cmp::min(msg_count as usize, MAX_TRANS_MSGS)
                    } else {
                        0
                    };
                    let msgs_ptr = unsafe { (*list_ptr).TransMsg.as_ptr() };

                    for i in 0..actual_count {
                        let msg = unsafe { &*msgs_ptr.add(i) };
                        let message = msg.message;
                        let wp = msg.wParam.0 as u32;
                        let lp = msg.lParam.0 as u32;
                        if message == 0 {
                            continue;
                        }
                        if message == WM_IME_COMPOSITION && (lp & GCS_RESULTSTR.0 as u32) != 0 {
                            has_commit_msg = true;
                        }
                        if message == WM_IME_CHAR || message == WM_CHAR {
                            if let Some(ch) = std::char::from_u32(wp & 0xFFFF) {
                                char_commits.push(ch);
                            }
                        }
                    }

                    // Phase 2: try reading GCS_RESULTSTR (may be empty if IME cleared it)
                    if has_commit_msg {
                        if let Some(rstr) = crate::win_imm::imm_ops::get_result_string(session.himc)
                        {
                            let mut s = session.pending_commit.take().unwrap_or_default();
                            s.push_str(&rstr);
                            session.pending_commit = Some(s);
                            tracing::info!("commit: '{}'", rstr);
                        } else {
                            // SogouPY does not populate COMPOSITIONSTRING.dwResultStr
                            // for standalone punctuation (no prior composition).
                            // Fall back to our Chinese punctuation mapping table.
                            if self.enable_punct_fallback {
                                if let Some(punct) =
                                    crate::win_imm::punct_map::map_punctuation(ascii_code)
                                {
                                    let mut s = session.pending_commit.take().unwrap_or_default();
                                    s.push_str(&punct);
                                    session.pending_commit = Some(s);
                                    tracing::info!("commit (punct fallback): '{}'", punct);
                                } else {
                                    tracing::warn!("GCS_RESULTSTR empty and no punct mapping for ascii_code=0x{:X}", ascii_code);
                                }
                            } else {
                                tracing::warn!("GCS_RESULTSTR empty and punct fallback disabled for ascii=0x{:X}", ascii_code);
                            }
                        }
                    }

                    // Phase 3: pump WM_CHAR from message queue
                    unsafe {
                        let mut msg_buf: MSG = std::mem::zeroed();
                        let mut wm_chars = String::new();
                        while PeekMessageW(&mut msg_buf, session.hwnd, WM_CHAR, WM_CHAR, PM_REMOVE)
                            .as_bool()
                        {
                            if let Some(ch) = std::char::from_u32(msg_buf.wParam.0 as u32) {
                                wm_chars.push(ch);
                            }
                            if wm_chars.len() > 256 {
                                break;
                            }
                        }
                        if !wm_chars.is_empty() {
                            tracing::debug!("WM_CHAR queue: '{}'", wm_chars);
                            let mut s = session.pending_commit.take().unwrap_or_default();
                            s.push_str(&wm_chars);
                            session.pending_commit = Some(s);
                        }
                    }

                    // Phase 4: if no commit data from above, try char_commits
                    if session.pending_commit.is_none() && !char_commits.is_empty() {
                        session.pending_commit = Some(char_commits);
                    }

                    return true;
                }
            }
        }
        false
    }
}

impl Drop for ImmRimeAdapter {
    fn drop(&mut self) {
        {
            // First destroy all sessions properly
            for (_, session) in self.sessions.drain() {
                if let Some(ime) = &self.ime_functions {
                    unsafe {
                        let _ = (ime.select)(session.himc, windows::Win32::Foundation::BOOL(0));
                    }; // FALSE
                }
                session.destroy();
            }

            // We do not FreeLibrary here. DLL will be cleanly flushed on process exit.
        }
    }
}

#[tonic::async_trait]
impl RimeBackend for ImmRimeAdapter {
    async fn open_session(&mut self) -> Option<usize> {
        {
            let id = self.sessions.len() + 1; // Basic sequential ID
            if let Some(ime) = &self.ime_functions {
                match session::WinImmSession::create(id, ime.h_module, self.show_window) {
                    Ok(session) => {
                        // Activate it for the Input Context
                        let result = unsafe {
                            (ime.select)(session.himc, windows::Win32::Foundation::BOOL(1))
                        };
                        if result.as_bool() {
                            self.sessions.insert(id, session);
                            return Some(id);
                        } else {
                            session.destroy();
                        }
                    }
                    Err(_) => return None,
                }
            }
        }
        None
    }

    async fn destroy_session(&mut self, session_id: usize) {
        {
            if let Some(session) = self.sessions.remove(&session_id) {
                if let Some(ime) = &self.ime_functions {
                    unsafe {
                        let _ = (ime.select)(session.himc, windows::Win32::Foundation::BOOL(0));
                    }; // FALSE
                }
                session.destroy();
            }
        }
    }

    async fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool {
        let vk = crate::win_imm::vk_map::rime_to_vk(key.keycode);
        let rime_mod = key.modifier;

        // Rime's kReleaseMask is 1 << 30
        let is_keyup = (rime_mod & (1 << 30)) != 0;

        // Rime modifier masks: kShiftMask=1, kLockMask=2, kControlMask=4, kAltMask=8
        let is_alt = (rime_mod & 8) != 0;
        let is_ctrl = (rime_mod & 4) != 0;
        if (is_ctrl || is_alt) && !(is_ctrl && is_alt) {
            // Return false for plain Ctrl+X or Alt+X shortcuts
            return false;
        }

        let is_shift = (rime_mod & 1) != 0 || crate::win_imm::vk_map::is_shifted_char(key.keycode);

        // Convert to process_vk modifiers: 1=Shift, 2=Ctrl, 4=Alt
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

        self.process_vk(session_id, VIRTUAL_KEY(vk as u16), win_mod, is_keyup)
            .await
    }

    async fn get_context(&mut self, session_id: usize) -> RimeContextProto {
        {
            if let Some(session) = self.sessions.get(&session_id) {
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
        }

        // Fallback or empty struct
        RimeContextProto {
            composition: None,
            menu: None,
            commit_text_preview: String::new(),
        }
    }

    async fn get_commit(&mut self, session_id: usize) -> Option<String> {
        {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                return session.pending_commit.take();
            }
        }
        None
    }

    async fn select_candidate(&mut self, session_id: usize, index: usize) -> bool {
        {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                unsafe {
                    let _ = windows::Win32::UI::Input::Ime::ImmNotifyIME(
                        session.himc,
                        windows::Win32::UI::Input::Ime::NI_SELECTCANDIDATESTR,
                        windows::Win32::UI::Input::Ime::NOTIFY_IME_INDEX(0),
                        index as u32,
                    );
                    // Force checking if this produced a commit immediately
                    if let Some(rstr) = crate::win_imm::imm_ops::get_result_string(session.himc) {
                        session.pending_commit = Some(rstr);
                    }
                }
                return true;
            }
        }
        false
    }
}
