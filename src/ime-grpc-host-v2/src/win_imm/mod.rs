// src/win_imm/mod.rs
pub mod imm_ops;
pub mod keys;
pub mod session;
pub mod thread_pump;
pub mod channel_adapter;

use std::collections::HashMap;
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

impl ImmRimeAdapter {
    pub fn new() -> Self {
        #[cfg(windows)]
        let ime_functions = {
            // Hardcoded to QQPinyin for now, as requested.
            use windows::core::w;
            match crate::win_imm::imm_ops::load_ime_dll(w!("C:\\windows\\system32\\QQPinyin.ime")) {
                Ok(funcs) => {
                    // Global initialize
                    unsafe {
                        let mut ime_info = std::mem::zeroed();
                        let mut class_name = [0u16; 256];
                        (funcs.inquire)(&mut ime_info, class_name.as_mut_ptr(), 0);
                    }
                    Some(funcs)
                },
                Err(e) => {
                    println!("Failed to load QQPinyin.ime: {:?}", e);
                    None
                }
            }
        };

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
                    unsafe { (ime.select)(session.himc, windows::Win32::Foundation::BOOL(0)) }; // FALSE
                }
                session.destroy();
            }

            // Unload library as a final step
            if let Some(ime) = self.ime_functions.take() {
                unsafe {
                    let _ = FreeLibrary(ime.h_module);
                }
            }
        }
    }
}

impl RimeBackend for ImmRimeAdapter {
    fn open_session(&mut self) -> Option<usize> {
        #[cfg(windows)]
        {
            let id = self.sessions.len() + 1; // Basic sequential ID
            if let Some(ime) = &self.ime_functions {
                match session::WinImmSession::create(id, ime.h_module) {
                    Ok(mut session) => {
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

    fn destroy_session(&mut self, session_id: usize) {
        #[cfg(windows)]
        {
            if let Some(mut session) = self.sessions.remove(&session_id) {
                if let Some(ime) = &self.ime_functions {
                    unsafe { (ime.select)(session.himc, windows::Win32::Foundation::BOOL(0)) }; // FALSE
                }
                session.destroy();
            }
        }
    }

    fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool {
        #[cfg(windows)]
        {
            if let Some(session) = self.sessions.get_mut(&session_id) {
                if let Some(ime) = &self.ime_functions {
                    // Extract modifier information (assuming 0x1=Shift, 0x2=Ctrl, 0x4=Alt)
                    // Populate lpbKeyState
                    let mut key_state = [0u8; 256];
                    let vk = key.keycode;
                    let modifiers = key.modifier;

                    // Rough mapping (will improve later with constants)
                    if (modifiers & 1) != 0 { key_state[0x10] = 0x80; } // VK_SHIFT
                    if (modifiers & 2) != 0 { key_state[0x11] = 0x80; } // VK_CONTROL
                    if (modifiers & 4) != 0 { key_state[0x12] = 0x80; } // VK_MENU

                    let is_consumed = unsafe {
                        (ime.process_key)(
                            session.himc,
                            vk,
                            0, // lKeyData
                            key_state.as_ptr(),
                        )
                    };

                    if is_consumed.as_bool() {
                        let mut trans_key = 0u32;
                        unsafe {
                            (ime.to_ascii_ex)(
                                vk,
                                0, // scancode
                                key_state.as_ptr(),
                                &mut trans_key,
                                0, // fuState
                                session.himc,
                            );
                        }
                        return true;
                    }
                }
            }
        }
        false
    }

    fn get_context(&mut self, session_id: usize) -> RimeContextProto {
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

    fn get_commit(&mut self, session_id: usize) -> Option<String> {
        #[cfg(windows)]
        {
            if let Some(session) = self.sessions.get(&session_id) {
                return crate::win_imm::imm_ops::get_result_string(session.himc);
            }
        }
        None
    }
}
