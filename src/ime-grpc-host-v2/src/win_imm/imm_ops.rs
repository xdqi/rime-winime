use windows::core::{s, PCWSTR};
use windows::Win32::Foundation::{BOOL, HMODULE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Input::Ime::{
    ImmGenerateMessage, ImmGetCandidateListW, ImmGetCompositionStringW, ImmSetConversionStatus,
    ATTR_TARGET_CONVERTED, ATTR_TARGET_NOTCONVERTED, CANDIDATELIST, GCS_COMPATTR, GCS_COMPSTR,
    GCS_CURSORPOS, GCS_RESULTSTR, HIMC, IME_CMODE_NATIVE, TRANSMSGLIST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
use windows::Win32::UI::WindowsAndMessaging::GetWindowTextW;

#[link(name = "imm32")]
extern "system" {
    fn ImmProcessKey(hwnd: HWND, hkl: HKL, vk: u32, lparam: LPARAM, hotkey: u32) -> u32;
    fn ImmTranslateMessage(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> BOOL;
}

pub type PImeInquire = unsafe extern "system" fn(
    lp_imeinfo: *mut windows::Win32::UI::Input::Ime::IMEINFO,
    lpsz_wnd_class: *mut u16,
    dw_system_info_flags: u32,
) -> BOOL;
pub type PImeSelect = unsafe extern "system" fn(h_imc: HIMC, f_select: BOOL) -> BOOL;
pub type PImeProcessKey = unsafe extern "system" fn(
    h_imc: HIMC,
    v_key: u32,
    l_key_data: u32,
    lpb_key_state: *const u8,
) -> BOOL;
pub type PImeToAsciiEx = unsafe extern "system" fn(
    u_vkey: u32,
    u_scan_code: u32,
    lpb_key_state: *const u8,
    lp_trans_msg_list: *mut TRANSMSGLIST,
    fu_state: u32,
    h_imc: HIMC,
) -> u32;
pub type PImeNotifyIME =
    unsafe extern "system" fn(h_imc: HIMC, action: u32, index: u32, value: u32) -> BOOL;
pub type PImeSetActiveContext = unsafe extern "system" fn(h_imc: HIMC, f_activate: BOOL) -> BOOL;

pub fn imm_generate_message(himc: HIMC) -> bool {
    unsafe { ImmGenerateMessage(himc).as_bool() }
}

pub fn imm_set_native_conversion_status(himc: HIMC) -> bool {
    unsafe { ImmSetConversionStatus(himc, IME_CMODE_NATIVE, Default::default()).as_bool() }
}

pub fn imm_process_key(hwnd: HWND, hkl: HKL, vk: u32, lparam: u32) -> u32 {
    unsafe { ImmProcessKey(hwnd, hkl, vk, LPARAM(lparam as isize), u32::MAX) }
}

pub fn imm_translate_message(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: u32,
) -> bool {
    unsafe { ImmTranslateMessage(hwnd, message, WPARAM(wparam), LPARAM(lparam as isize)).as_bool() }
}

pub fn get_window_text(hwnd: HWND) -> String {
    let mut buf = vec![0u16; 1024];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        String::new()
    } else {
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

#[derive(Clone, Copy)]
pub struct ImeFunctions {
    pub h_module: HMODULE,
    pub inquire: PImeInquire,
    pub select: PImeSelect,
    pub process_key: PImeProcessKey,
    pub to_ascii_ex: PImeToAsciiEx,
    pub notify_ime: Option<PImeNotifyIME>,
    pub set_active_context: Option<PImeSetActiveContext>,
}

unsafe impl Send for ImeFunctions {}
unsafe impl Sync for ImeFunctions {}

pub fn load_ime_dll(path: PCWSTR) -> Result<ImeFunctions, windows::core::Error> {
    unsafe {
        let h_module = LoadLibraryW(path)?;

        let inquire_ptr = GetProcAddress(h_module, s!("ImeInquire")).expect("ImeInquire not found");
        let select_ptr = GetProcAddress(h_module, s!("ImeSelect")).expect("ImeSelect not found");
        let process_key_ptr =
            GetProcAddress(h_module, s!("ImeProcessKey")).expect("ImeProcessKey not found");
        let to_ascii_ex_ptr =
            GetProcAddress(h_module, s!("ImeToAsciiEx")).expect("ImeToAsciiEx not found");
        let notify_ime_ptr = GetProcAddress(h_module, s!("NotifyIME"));
        let set_active_context_ptr = GetProcAddress(h_module, s!("ImeSetActiveContext"));

        Ok(ImeFunctions {
            h_module,
            inquire: std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(
                    *mut windows::Win32::UI::Input::Ime::IMEINFO,
                    *mut u16,
                    u32,
                ) -> windows::Win32::Foundation::BOOL,
            >(inquire_ptr),
            select: std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(
                    windows::Win32::UI::Input::Ime::HIMC,
                    windows::Win32::Foundation::BOOL,
                ) -> windows::Win32::Foundation::BOOL,
            >(select_ptr),
            process_key: std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(
                    windows::Win32::UI::Input::Ime::HIMC,
                    u32,
                    u32,
                    *const u8,
                ) -> windows::Win32::Foundation::BOOL,
            >(process_key_ptr),
            to_ascii_ex: std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn(
                    u32,
                    u32,
                    *const u8,
                    *mut windows::Win32::UI::Input::Ime::TRANSMSGLIST,
                    u32,
                    windows::Win32::UI::Input::Ime::HIMC,
                ) -> u32,
            >(to_ascii_ex_ptr),
            notify_ime: notify_ime_ptr.map(|ptr| {
                std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn(
                        windows::Win32::UI::Input::Ime::HIMC,
                        u32,
                        u32,
                        u32,
                    ) -> windows::Win32::Foundation::BOOL,
                >(ptr)
            }),
            set_active_context: set_active_context_ptr.map(|ptr| {
                std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn(
                        windows::Win32::UI::Input::Ime::HIMC,
                        windows::Win32::Foundation::BOOL,
                    ) -> windows::Win32::Foundation::BOOL,
                >(ptr)
            }),
        })
    }
}

pub struct CompositionData {
    pub text: String,
    pub cursor_pos: i32,
    pub sel_start: i32,
    pub sel_end: i32,
}

pub fn get_composition_string(himc: HIMC) -> Option<CompositionData> {
    unsafe {
        let size = ImmGetCompositionStringW(himc, GCS_COMPSTR, None, 0);
        if size <= 0 {
            return None;
        }

        let mut buffer: Vec<u16> = vec![0; (size / 2) as usize];
        let bytes_copied = ImmGetCompositionStringW(
            himc,
            GCS_COMPSTR,
            Some(buffer.as_mut_ptr() as *mut _),
            size as u32,
        );

        if bytes_copied > 0 {
            let u16_len = (bytes_copied / 2) as usize;
            let actual_buffer = &buffer[0..u16_len];
            let text = String::from_utf16_lossy(actual_buffer);

            let utf16_to_utf8_offset = |u16_idx: usize| -> i32 {
                let limited = &actual_buffer[0..usize::min(u16_idx, actual_buffer.len())];
                String::from_utf16_lossy(limited).len() as i32
            };

            // Get cursor pos, which IMM returns in characters natively!
            let mut cursor_pos = ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0);
            if cursor_pos < 0 {
                cursor_pos = actual_buffer.len() as i32; // fallback to the end
            }
            let u8_cursor_pos = utf16_to_utf8_offset(cursor_pos as usize);

            // GCS_COMPATTR
            let attr_size = ImmGetCompositionStringW(himc, GCS_COMPATTR, None, 0);
            let mut u16_sel_start = 0;
            let mut u16_sel_end = actual_buffer.len() as i32;

            if attr_size > 0 {
                let mut attr_buffer: Vec<u8> = vec![0; attr_size as usize];
                if ImmGetCompositionStringW(
                    himc,
                    GCS_COMPATTR,
                    Some(attr_buffer.as_mut_ptr() as *mut _),
                    attr_size as u32,
                ) > 0
                {
                    let mut start = -1;
                    let mut end = -1;
                    for (i, &attr) in attr_buffer.iter().enumerate() {
                        if attr == ATTR_TARGET_CONVERTED as u8
                            || attr == ATTR_TARGET_NOTCONVERTED as u8
                        {
                            if start == -1 {
                                start = i as i32;
                            }
                            end = (i + 1) as i32;
                        }
                    }
                    if start != -1 {
                        u16_sel_start = start;
                        u16_sel_end = end;
                    }
                }
            }

            let u8_sel_start = utf16_to_utf8_offset(u16_sel_start as usize);
            let u8_sel_end = utf16_to_utf8_offset(u16_sel_end as usize);

            Some(CompositionData {
                text,
                cursor_pos: u8_cursor_pos,
                sel_start: u8_sel_start,
                sel_end: u8_sel_end,
            })
        } else {
            None
        }
    }
}

pub fn get_result_string(himc: HIMC) -> Option<String> {
    unsafe {
        let size = ImmGetCompositionStringW(himc, GCS_RESULTSTR, None, 0);
        if size <= 0 {
            return None;
        }

        let mut buffer: Vec<u16> = vec![0; (size / 2) as usize];
        let bytes_copied = ImmGetCompositionStringW(
            himc,
            GCS_RESULTSTR,
            Some(buffer.as_mut_ptr() as *mut _),
            size as u32,
        );
        if bytes_copied > 0 {
            Some(String::from_utf16_lossy(&buffer))
        } else {
            None
        }
    }
}

pub fn get_candidate_list(himc: HIMC) -> Option<crate::proto::rime_service_v2::MenuProto> {
    unsafe {
        let size = ImmGetCandidateListW(himc, 0, None, 0);
        if size == 0 {
            return None;
        }

        let mut buffer: Vec<u8> = vec![0; size as usize];
        let p_cand_list = buffer.as_mut_ptr() as *mut CANDIDATELIST;

        let bytes_copied = ImmGetCandidateListW(himc, 0, Some(p_cand_list), size);
        if bytes_copied == 0 {
            return None;
        }

        let cand_list = &*p_cand_list;
        let count = cand_list.dwCount as usize;
        if count == 0 {
            return None;
        }

        let mut candidates = Vec::new();

        // The dwOffset array starts right after the fixed header fields.
        // Some IMEs (e.g. Sogou on Wine) write zeros at the standard dwOffset
        // positions and place the real offsets further into the buffer.
        // Scan forward from dwOffset[0] to find the first non-zero value that
        // looks like a valid string offset into the buffer.
        let max_offset_slots = (size as usize - std::mem::offset_of!(CANDIDATELIST, dwOffset)) / 4;
        let all_offsets = std::slice::from_raw_parts(cand_list.dwOffset.as_ptr(), max_offset_slots);

        let mut base_idx = 0usize;
        if all_offsets.first().copied().unwrap_or(0) == 0 {
            // dwOffset[0] is zero — scan for the real offset array
            for (idx, &val) in all_offsets.iter().enumerate() {
                if val > 0 && val < size as u32 {
                    base_idx = idx;
                    break;
                }
            }
        }

        for i in 0..count {
            let slot = base_idx + i;
            if slot >= max_offset_slots {
                break;
            }
            let offset = all_offsets[slot] as usize;
            if offset == 0 || offset >= size as usize {
                continue;
            }
            let string_ptr = buffer.as_ptr().add(offset) as *const u16;
            let max_chars = (size as usize - offset) / 2;

            let mut len = 0;
            while len < max_chars && *string_ptr.add(len) != 0 {
                len += 1;
            }

            let text = String::from_utf16_lossy(std::slice::from_raw_parts(string_ptr, len));

            candidates.push(crate::proto::rime_service_v2::CandidateProto {
                text,
                comment: String::new(),
                quality: 0.0,
            });
        }

        let page_size = if cand_list.dwPageSize > 0 {
            cand_list.dwPageSize as i32
        } else {
            10
        };
        let page_no = if page_size > 0 {
            (cand_list.dwPageStart as i32) / page_size
        } else {
            0
        };

        Some(crate::proto::rime_service_v2::MenuProto {
            candidates,
            page_size,
            page_no,
            is_last_page: false,
            highlighted_candidate_index: cand_list.dwSelection as i32,
            num_candidates: cand_list.dwCount as i32,
            select_keys: String::new(),
        })
    }
}
