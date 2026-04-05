#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use windows::Win32::Foundation::{BOOL, HMODULE, HWND, LPARAM, WPARAM};
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
#[cfg(windows)]
use windows::Win32::UI::Input::Ime::{HIMC, ImmGetCompositionStringW, ImmGetCandidateListW, GCS_COMPSTR, GCS_RESULTSTR, GCS_CURSORPOS, CANDIDATELIST};
#[cfg(windows)]
use windows::core::{s, PCWSTR};

#[cfg(windows)]
pub type PImeInquire = unsafe extern "system" fn(lpIMEInfo: *mut windows::Win32::UI::Input::Ime::IMEINFO, lpszWndClass: *mut u16, dwSystemInfoFlags: u32) -> BOOL;
#[cfg(windows)]
pub type PImeSelect = unsafe extern "system" fn(hIMC: HIMC, fSelect: BOOL) -> BOOL;
#[cfg(windows)]
pub type PImeProcessKey = unsafe extern "system" fn(hIMC: HIMC, vKey: u32, lKeyData: u32, lpbKeyState: *const u8) -> BOOL;
#[cfg(windows)]
pub type PImeToAsciiEx = unsafe extern "system" fn(uVKey: u32, uScanCode: u32, lpbKeyState: *const u8, lpdwTransKey: *mut u32, fuState: u32, hIMC: HIMC) -> u32;

#[cfg(windows)]
#[derive(Clone, Copy)]
pub struct ImeFunctions {
    pub h_module: HMODULE,
    pub inquire: PImeInquire,
    pub select: PImeSelect,
    pub process_key: PImeProcessKey,
    pub to_ascii_ex: PImeToAsciiEx,
}

#[cfg(windows)]
unsafe impl Send for ImeFunctions {}
#[cfg(windows)]
unsafe impl Sync for ImeFunctions {}

#[cfg(windows)]
pub fn load_ime_dll(path: PCWSTR) -> Result<ImeFunctions, windows::core::Error> {
    unsafe {
        let h_module = LoadLibraryW(path)?;
        
        let inquire_ptr = GetProcAddress(h_module, s!("ImeInquire")).expect("ImeInquire not found");
        let select_ptr = GetProcAddress(h_module, s!("ImeSelect")).expect("ImeSelect not found");
        let process_key_ptr = GetProcAddress(h_module, s!("ImeProcessKey")).expect("ImeProcessKey not found");
        let to_ascii_ex_ptr = GetProcAddress(h_module, s!("ImeToAsciiEx")).expect("ImeToAsciiEx not found");

        Ok(ImeFunctions {
            h_module,
            inquire: std::mem::transmute(inquire_ptr),
            select: std::mem::transmute(select_ptr),
            process_key: std::mem::transmute(process_key_ptr),
            to_ascii_ex: std::mem::transmute(to_ascii_ex_ptr),
        })
    }
}

#[cfg(windows)]
pub struct CompositionData {
    pub text: String,
    pub cursor_pos: i32,
    pub sel_start: i32,
    pub sel_end: i32,
}

#[cfg(windows)]
pub fn get_composition_string(himc: HIMC) -> Option<CompositionData> {
    unsafe {
        let size = ImmGetCompositionStringW(himc, GCS_COMPSTR, None, 0);
        if size <= 0 {
            return None;
        }

        let mut buffer: Vec<u16> = vec![0; (size / 2) as usize];
        let bytes_copied = ImmGetCompositionStringW(himc, GCS_COMPSTR, Some(buffer.as_mut_ptr() as *mut _), size as u32);
        
        if bytes_copied > 0 {
            let text = String::from_utf16_lossy(&buffer);
            
            // Get cursor pos, which IMM returns in characters natively!
            let mut cursor_pos = ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0);
            if cursor_pos < 0 {
                cursor_pos = text.chars().count() as i32; // fallback to the end
            }

            Some(CompositionData {
                text,
                cursor_pos,
                sel_start: cursor_pos, // Map them to cursor_pos for now
                sel_end: cursor_pos,
            })
        } else {
            None
        }
    }
}

#[cfg(windows)]
pub fn get_result_string(himc: HIMC) -> Option<String> {
    unsafe {
        let size = ImmGetCompositionStringW(himc, GCS_RESULTSTR, None, 0);
        if size <= 0 {
            return None;
        }

        let mut buffer: Vec<u16> = vec![0; (size / 2) as usize];
        let bytes_copied = ImmGetCompositionStringW(himc, GCS_RESULTSTR, Some(buffer.as_mut_ptr() as *mut _), size as u32);
        if bytes_copied > 0 {
            Some(String::from_utf16_lossy(&buffer))
        } else {
            None
        }
    }
}

#[cfg(windows)]
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
        let mut candidates = Vec::new();
        
        // Ensure within bounds, dwOffset acts as the starting element of an array
        let offsets_ptr = cand_list.dwOffset.as_ptr();
        
        for i in 0..cand_list.dwCount {
            let offset = *offsets_ptr.add(i as usize);
            let string_ptr = buffer.as_ptr().add(offset as usize) as *const u16;
            
            let mut len = 0;
            while *string_ptr.add(len) != 0 {
                len += 1;
            }
            
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(string_ptr, len));
            
            candidates.push(crate::proto::rime_service_v2::CandidateProto {
                text,
                comment: String::new(),
                quality: 0.0,
            });
        }

        let page_size = if cand_list.dwPageSize > 0 { cand_list.dwPageSize as i32 } else { 10 };
        let page_no = if page_size > 0 { (cand_list.dwPageStart as i32) / page_size } else { 0 };

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
