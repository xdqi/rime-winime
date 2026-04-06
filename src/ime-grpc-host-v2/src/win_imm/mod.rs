// src/win_imm/mod.rs
pub mod imm_ops;
pub mod keys;
pub mod session;
pub mod thread_pump;
pub mod channel_adapter;
pub mod vk_map;

use std::collections::HashMap;
use std::sync::OnceLock;
use crate::win_imm::imm_ops::ImeFunctions;
use crate::backend::RimeBackend;

/// Known paired punctuation: maps opener to closer.
/// When the IME commits "opener+closer" as a single result string,
/// we split them: commit only the opener, store the closer to append to the next commit.
fn split_paired_punct(s: &str) -> Option<(&str, &str)> {
    // Each pair: (opener, closer) as &str
    const PAIRS: &[(&str, &str)] = &[
        ("\u{FF08}", "\u{FF09}"),   // （ ）
        ("\u{201C}", "\u{201D}"),   // " "
        ("\u{2018}", "\u{2019}"),   // ' '
        ("\u{3010}", "\u{3011}"),   // 【 】
        ("\u{300A}", "\u{300B}"),   // 《 》
        ("\u{300C}", "\u{300D}"),   // 「 」
        ("\u{300E}", "\u{300F}"),   // 『 』
        ("\u{3008}", "\u{3009}"),   // 〈 〉
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
    pub fn FreeLibrary(hLibModule: windows::Win32::Foundation::HMODULE) -> windows::Win32::Foundation::BOOL;
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
                    // Global initialize
                    unsafe {
                        let mut ime_info = std::mem::zeroed();
                        let mut class_name = [0u16; 256];
                        let _ = (funcs.inquire)(&mut ime_info, class_name.as_mut_ptr(), 0);
                    }
                    Some(funcs)
                },
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
}

impl Drop for ImmRimeAdapter {
    fn drop(&mut self) {
        {
            // First destroy all sessions properly
            for (_, session) in self.sessions.drain() {
                if let Some(ime) = &self.ime_functions {
                    unsafe { let _ = (ime.select)(session.himc, windows::Win32::Foundation::BOOL(0)); }; // FALSE
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
                        let result = unsafe { (ime.select)(session.himc, windows::Win32::Foundation::BOOL(1)) };
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
                    unsafe { let _ = (ime.select)(session.himc, windows::Win32::Foundation::BOOL(0)); }; // FALSE
                }
                session.destroy();
            }
        }
    }

    async fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool {
        {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                if let Some(ime) = &self.ime_functions {
                    // Extract modifier information
                    // Populate lpbKeyState
                    let mut key_state = [0u8; 256];
                    let vk = crate::win_imm::vk_map::rime_to_vk(key.keycode);
                    let modifiers = key.modifier;

                    // Rime's kReleaseMask is 1 << 30
                    let is_keyup = (modifiers & (1 << 30)) != 0;

                    // Rime's modifier masks from rime/key_table.h:
                    // kShiftMask=1, kLockMask=2, kControlMask=4, kAltMask/kMod1Mask=8
                    let is_alt = (modifiers & 8) != 0;
                    let is_ctrl = (modifiers & 4) != 0;
                    if (is_ctrl || is_alt) && !(is_ctrl && is_alt) {
                        // Return false for plain Ctrl+X or Alt+X shortcuts, avoiding IME blocking Fcitx shortcuts
                        return false;
                    }

                    let is_shift = (modifiers & 1) != 0 || crate::win_imm::vk_map::is_shifted_char(key.keycode);
                    
                    if is_shift {
                        key_state[0x10] = 0x80; // VK_SHIFT
                        key_state[0xA0] = 0x80; // VK_LSHIFT
                    }
                    if (modifiers & 4) != 0 {
                        key_state[0x11] = 0x80; // VK_CONTROL
                        key_state[0xA2] = 0x80; // VK_LCONTROL
                    }
                    if is_alt {
                        key_state[0x12] = 0x80; // VK_MENU (Alt)
                        key_state[0xA4] = 0x80; // VK_LMENU
                    }

                    let l_key_data = crate::win_imm::vk_map::make_l_key_data(vk, is_keyup, is_alt);

                    tracing::info!(
                        "process_key input: keycode=0x{:X}, modifiers={}, is_keyup={}, is_shift={}, is_ctrl={}, is_alt={} => vk=0x{:X}",
                        key.keycode, modifiers, is_keyup, is_shift, is_ctrl, is_alt, vk
                    );

                    let is_consumed = unsafe {
                        windows::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState(&key_state).ok();
                        let res = (ime.process_key)(
                            session.himc,
                            vk,
                            l_key_data,
                            key_state.as_ptr(),
                        );
                        // Windows typically restores keyboard state later or doesn't care
                        res
                    };
                    tracing::info!("ImeProcessKey returned: {:?}", is_consumed);

                    if is_consumed.as_bool() {
                        // The buffer starts with a count of how many messages it can hold
                        // We allocate 1 + 3 * 256 words
                        let mut trans_msgs = vec![0u32; 1 + 3 * 256];
                        trans_msgs[0] = 256; // max messages
                        
                        let msg_count = unsafe {
                            (ime.to_ascii_ex)(
                                vk,
                                0, // scancode
                                key_state.as_ptr(),
                                trans_msgs.as_mut_ptr(),
                                0, // fuState
                                session.himc,
                            )
                        } as i32;
                        tracing::info!("ToAsciiEx returned translation message count: {}", msg_count);

                        let mut has_commit_msg = false;
                        let mut char_commits = String::new();
                        if msg_count > 0 && msg_count <= 256 {
                            for i in 0..(msg_count as usize) {
                                // index 0 is capacity/count, actual array starts at 1
                                let msg_base = 1 + i * 3;
                                let message = trans_msgs[msg_base];
                                let wparam = trans_msgs[msg_base + 1];
                                let lparam = trans_msgs[msg_base + 2];
                                tracing::info!("Trans msg: 0x{:X}, wp: 0x{:X}, lp: 0x{:X}", message, wparam, lparam);
                                if message == 0x010F /* WM_IME_COMPOSITION */
                                    && (lparam & 0x0800 /* GCS_RESULTSTR */) != 0 {
                                    has_commit_msg = true;
                                }
                                if message == 0x0286 /* WM_IME_CHAR */ || message == 0x0102 /* WM_CHAR */ {
                                    // Collect characters generated by the IME
                                    if let Some(ch) = std::char::from_u32(wparam & 0xFFFF) {
                                        char_commits.push(ch);
                                        tracing::info!("Collected WM_(IME)_CHAR: 0x{:X} -> {}", wparam, ch);
                                    }
                                }
                            }
                        }
                        
                        if has_commit_msg {
                            if let Some(rstr) = crate::win_imm::imm_ops::get_result_string(session.himc) {
                                let mut s = session.pending_commit.take().unwrap_or_default();
                                s.push_str(&rstr);
                                session.pending_commit = Some(s);
                                tracing::info!("Collected GCS_RESULTSTR: {}", rstr);
                            } else {
                                tracing::warn!("GCS_RESULTSTR flag was set but get_result_string returned None");
                            }
                        } else if !char_commits.is_empty() {
                            let mut s = session.pending_commit.take().unwrap_or_default();
                            s.push_str(&char_commits);
                            session.pending_commit = Some(s);
                        }
                        // Probe: pump the hidden window's message queue to see if the IME
                        // injected any keybd_event (e.g. VK_LEFT for paired brackets)
                        unsafe {
                            let mut msg: windows::Win32::UI::WindowsAndMessaging::MSG = std::mem::zeroed();
                            let mut queue_msgs = Vec::new();
                            while windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                                &mut msg,
                                session.hwnd,
                                0, 0,
                                windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
                            ).as_bool() {
                                queue_msgs.push((msg.message, msg.wParam.0 as u32, msg.lParam.0 as u32));
                                // Prevent infinite loop: limit to 32 messages
                                if queue_msgs.len() >= 32 { break; }
                            }
                            // Also check thread messages (HWND(0))
                            while windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                                &mut msg,
                                windows::Win32::Foundation::HWND(0 as _),
                                0, 0,
                                windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
                            ).as_bool() {
                                queue_msgs.push((msg.message, msg.wParam.0 as u32, msg.lParam.0 as u32));
                                if queue_msgs.len() >= 64 { break; }
                            }
                            if !queue_msgs.is_empty() {
                                for (m, wp, lp) in &queue_msgs {
                                    tracing::info!("Queued msg: 0x{:X}, wp: 0x{:X}, lp: 0x{:X}", m, wp, lp);
                                }
                            } else {
                                tracing::info!("No queued messages after ImeToAsciiEx");
                            }
                        }
                        return true;
                    }
                }
            }
        }
        false
    }

    async fn get_context(&mut self, session_id: usize) -> RimeContextProto {
        {
            if let Some(session) = self.sessions.get(&session_id) {
                let comp_str = crate::win_imm::imm_ops::get_composition_string(session.himc);
                let mut menu_proto = crate::win_imm::imm_ops::get_candidate_list(session.himc);

                // Set select_keys based on Sogou's internal mode flag (hPrivate offset 4)
                if let Some(ref mut menu) = menu_proto {
                    let n = menu.page_size as usize;
                    let mode_flag = crate::win_imm::imm_ops::get_sogou_mode_flag(session.himc);
                    let is_vmode = mode_flag
                        .map(|f| f == crate::win_imm::imm_ops::SOGOU_MODE_VMODE)
                        .unwrap_or(false);
                    if is_vmode {
                        menu.select_keys = "abcdefghij"[..n.min(10)].to_string();
                    } else {
                        menu.select_keys = "1234567890"[..n.min(10)].to_string();
                    }

                    // Assert mode detection consistency with preedit
                    if let Some(ref comp_data) = comp_str {
                        let preedit = &comp_data.text;
                        if preedit.starts_with('v') && preedit.len() > 1 {
                            let rest = &preedit[1..];
                            let is_numeric_pattern = rest.chars().all(|c| c.is_ascii_digit() || c == '.');
                            let is_alpha_pattern = rest.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false);
                            tracing::info!(
                                "select_keys check: preedit='{}', mode_flag={:#X?}, is_vmode={}, select_keys='{}'",
                                preedit, mode_flag, is_vmode, menu.select_keys
                            );
                            if is_numeric_pattern {
                                assert!(is_vmode, "v-mode preedit '{}' but mode_flag={:#X?}", preedit, mode_flag);
                            } else if is_alpha_pattern {
                                assert!(!is_vmode, "pinyin preedit '{}' but mode_flag={:#X?}", preedit, mode_flag);
                            }
                        }
                    }
                }

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
                        index as u32
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
