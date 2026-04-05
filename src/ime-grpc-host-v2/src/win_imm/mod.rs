// src/win_imm/mod.rs
pub mod imm_ops;
pub mod keys;
pub mod session;
pub mod thread_pump;
pub mod channel_adapter;
pub mod vk_map;

use std::collections::HashMap;
#[cfg(windows)]
use std::sync::OnceLock;
#[cfg(windows)]
use crate::win_imm::imm_ops::ImeFunctions;
use crate::backend::RimeBackend;
use crate::proto::rime_service_v2::{KeyEvent, RimeContextProto};

#[cfg(windows)]
extern "system" {
    pub fn FreeLibrary(hLibModule: windows::Win32::Foundation::HMODULE) -> windows::Win32::Foundation::BOOL;
}

/// The Win32 adapter that implements the RimeBackend trait.
pub struct ImmRimeAdapter {
    #[cfg(windows)]
    ime_functions: Option<ImeFunctions>,
    #[cfg(windows)]
    sessions: HashMap<usize, session::WinImmSession>,
}

#[cfg(windows)]
static IME_FUNCS: OnceLock<Option<ImeFunctions>> = OnceLock::new();

impl Default for ImmRimeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImmRimeAdapter {
    pub fn new() -> Self {
        #[cfg(windows)]
        let ime_functions = *IME_FUNCS.get_or_init(|| {
            // Hardcoded to QQPinyin for now, as requested.
            use windows::core::w;
            match crate::win_imm::imm_ops::load_ime_dll(w!("C:\\windows\\system32\\QQPinyin.ime")) {
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
                    println!("Failed to load QQPinyin.ime: {:?}", e);
                    None
                }
            }
        });

        Self {
            #[cfg(windows)]
            ime_functions,
            #[cfg(windows)]
            sessions: HashMap::new(),
            // Other initialization goes here
        }
    }
}

impl Drop for ImmRimeAdapter {
    fn drop(&mut self) {
        #[cfg(windows)]
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
        #[cfg(windows)]
        {
            let id = self.sessions.len() + 1; // Basic sequential ID
            if let Some(ime) = &self.ime_functions {
                match session::WinImmSession::create(id, ime.h_module) {
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
        #[cfg(windows)]
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
        #[cfg(windows)]
        {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                if let Some(ime) = &self.ime_functions {
                    // Extract modifier information
                    // Populate lpbKeyState
                    let mut key_state = [0u8; 256];
                    let vk = crate::win_imm::vk_map::rime_to_vk(key.keycode);
                    let modifiers = key.modifier;

                    let is_keyup = (modifiers & (1 << 14)) != 0;

                    // Rough mapping (will improve later with constants)
                    if (modifiers & 1) != 0 { key_state[0x10] = 0x80; } // VK_SHIFT
                    if (modifiers & 2) != 0 { key_state[0x11] = 0x80; } // VK_CONTROL
                    if (modifiers & 4) != 0 { key_state[0x12] = 0x80; } // VK_MENU

                    let l_key_data = crate::win_imm::vk_map::make_l_key_data(vk, is_keyup);

                    let is_consumed = unsafe {
                        (ime.process_key)(
                            session.himc,
                            vk,
                            l_key_data,
                            key_state.as_ptr(),
                        )
                    };

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

                        let mut has_commit_msg = false;
                        if msg_count > 0 && msg_count <= 256 {
                            for i in 0..(msg_count as usize) {
                                // index 0 is capacity/count, actual array starts at 1
                                let msg_base = 1 + i * 3;
                                let message = trans_msgs[msg_base];
                                let _wparam = trans_msgs[msg_base + 1];
                                let lparam = trans_msgs[msg_base + 2];
                                if message == 0x010F /* WM_IME_COMPOSITION */
                                    && (lparam & 0x0800 /* GCS_RESULTSTR */) != 0 {
                                        has_commit_msg = true;
                                    }
                            }
                        }
                        
                        if has_commit_msg {
                            if let Some(rstr) = crate::win_imm::imm_ops::get_result_string(session.himc) {
                                session.pending_commit = Some(rstr);
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
        #[cfg(windows)]
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
                        length: comp_data.text.chars().count() as i32,
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
        #[cfg(windows)]
        {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                return session.pending_commit.take();
            }
        }
        None
    }

    async fn select_candidate(&mut self, session_id: usize, index: usize) -> bool {
        #[cfg(windows)]
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
