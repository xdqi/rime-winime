use windows::core::GUID;
use windows::core::PWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;

const CTF_TIP_KEY: &str = "SOFTWARE\\Microsoft\\CTF\\TIP";
const CLSID_KEY: &str = "SOFTWARE\\Classes\\CLSID";

#[derive(Debug, Clone)]
pub struct TipInfo {
    pub clsid: GUID,
    pub description: String,
    pub dll_path: String,
    pub lang_id: u16,
    pub profile_guid: GUID,
}

#[derive(Clone, Copy)]
struct KnownTipProfile {
    clsid: GUID,
    lang_id: u16,
    profile_guid: GUID,
}

// Some installers under Wine register the COM class but fail to populate
// HKLM\\Software\\Microsoft\\CTF\\TIP\\...\\LanguageProfile. Keep a tiny
// built-in map for known TIPs whose profile GUID is fixed in upstream source.
const KNOWN_TIP_PROFILES: &[KnownTipProfile] = &[KnownTipProfile {
    // Google Japanese Input: src/win32/base/tsf_profile.cc
    clsid: GUID::from_u128(0xD5A86FD5_5308_47EA_AD16_9C4EB160EC3C),
    lang_id: 0x0411,
    profile_guid: GUID::from_u128(0x773EB24E_CA1D_4B1B_B420_FA985BB0B80D),
}];

fn apply_known_tip_profile_fallback(tip: &mut TipInfo) {
    if let Some(known) = KNOWN_TIP_PROFILES
        .iter()
        .find(|known| known.clsid == tip.clsid)
    {
        if tip.lang_id == 0 || tip.lang_id == 0x0804 {
            tip.lang_id = known.lang_id;
        }
        if tip.profile_guid == GUID::zeroed() {
            tip.profile_guid = known.profile_guid;
        }
    }
}

/// Read a REG_SZ value from an open registry key.
unsafe fn read_reg_string(hkey: HKEY, value_name: &str) -> Option<String> {
    let mut buf_len: u32 = 1024;
    let mut buf = vec![0u16; buf_len as usize];
    let mut reg_type = REG_VALUE_TYPE(0);

    let result = if value_name.is_empty() {
        RegQueryValueExW(
            hkey,
            windows::core::PCWSTR::null(),
            None,
            Some(&mut reg_type),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_len),
        )
    } else {
        let name_wide: Vec<u16> = value_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        RegQueryValueExW(
            hkey,
            windows::core::PCWSTR(name_wide.as_ptr()),
            None,
            Some(&mut reg_type),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_len),
        )
    };

    if result != ERROR_SUCCESS || reg_type != REG_SZ {
        return None;
    }

    let char_count = (buf_len / 2) as usize;
    if char_count == 0 {
        return None;
    }
    // Strip trailing null
    let end = if buf[char_count - 1] == 0 {
        char_count - 1
    } else {
        char_count
    };
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Parse a GUID string like "{86598FB9-66A2-463E-B9C2-AEB906D477AD}" into a GUID.
pub fn parse_guid(s: &str) -> Option<GUID> {
    let s = s.trim();
    if !s.starts_with('{') || !s.ends_with('}') {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    let parts: Vec<&str> = inner.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let data1 = u32::from_str_radix(parts[0], 16).ok()?;
    let data2 = u16::from_str_radix(parts[1], 16).ok()?;
    let data3 = u16::from_str_radix(parts[2], 16).ok()?;

    let g4 = parts[3];
    let g5 = parts[4];
    if g4.len() != 4 || g5.len() != 12 {
        return None;
    }
    let mut data4 = [0u8; 8];
    for i in 0..2 {
        data4[i] = u8::from_str_radix(&g4[i * 2..i * 2 + 2], 16).ok()?;
    }
    for i in 0..6 {
        data4[2 + i] = u8::from_str_radix(&g5[i * 2..i * 2 + 2], 16).ok()?;
    }

    Some(GUID::from_values(data1, data2, data3, data4))
}

/// Format a GUID for registry paths (uppercase hex).
pub fn guid_to_registry_string(g: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1,
        g.data2,
        g.data3,
        g.data4[0],
        g.data4[1],
        g.data4[2],
        g.data4[3],
        g.data4[4],
        g.data4[5],
        g.data4[6],
        g.data4[7],
    )
}

/// Read the InprocServer32 DLL path for a given CLSID.
unsafe fn get_inproc_server32(clsid: &GUID) -> Option<String> {
    let clsid_str = guid_to_registry_string(clsid);
    let subkey = format!("{}\\{}", CLSID_KEY, clsid_str);
    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut hkey: HKEY = HKEY::default();

    if RegOpenKeyExW(
        HKEY_LOCAL_MACHINE,
        windows::core::PCWSTR(subkey_wide.as_ptr()),
        0,
        KEY_READ,
        &mut hkey,
    )
    .0 != 0
    {
        return None;
    }

    let inproc_wide: Vec<u16> = "InprocServer32"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut hkey_inproc: HKEY = HKEY::default();

    if RegOpenKeyExW(
        hkey,
        windows::core::PCWSTR(inproc_wide.as_ptr()),
        0,
        KEY_READ,
        &mut hkey_inproc,
    )
    .0 != 0
    {
        RegCloseKey(hkey).ok().ok();
        return None;
    }

    let result = read_reg_string(hkey_inproc, "");
    RegCloseKey(hkey_inproc).ok().ok();
    RegCloseKey(hkey).ok().ok();
    result
}

unsafe fn get_clsid_description(clsid: &GUID) -> Option<String> {
    let clsid_str = guid_to_registry_string(clsid);
    let subkey = format!("{}\\{}", CLSID_KEY, clsid_str);
    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut hkey: HKEY = HKEY::default();

    if RegOpenKeyExW(
        HKEY_LOCAL_MACHINE,
        windows::core::PCWSTR(subkey_wide.as_ptr()),
        0,
        KEY_READ,
        &mut hkey,
    )
    .0 != 0
    {
        return None;
    }

    let result = read_reg_string(hkey, "");
    RegCloseKey(hkey).ok().ok();
    result
}

unsafe fn discover_tip_clsid_fallbacks() -> Vec<TipInfo> {
    let mut tips = Vec::new();
    let clsid_key_wide: Vec<u16> = CLSID_KEY.encode_utf16().chain(std::iter::once(0)).collect();
    let mut hkey_clsid: HKEY = HKEY::default();

    if RegOpenKeyExW(
        HKEY_LOCAL_MACHINE,
        windows::core::PCWSTR(clsid_key_wide.as_ptr()),
        0,
        KEY_READ,
        &mut hkey_clsid,
    )
    .0 != 0
    {
        return tips;
    }

    for idx in 0.. {
        let mut clsid_name = [0u16; 128];
        let mut clsid_name_len: u32 = clsid_name.len() as u32;

        let result = RegEnumKeyExW(
            hkey_clsid,
            idx,
            PWSTR(clsid_name.as_mut_ptr()),
            &mut clsid_name_len,
            None,
            PWSTR::null(),
            None,
            None,
        );

        if result != ERROR_SUCCESS {
            break;
        }

        let clsid_str = String::from_utf16_lossy(&clsid_name[..clsid_name_len as usize]);
        let Some(clsid) = parse_guid(&clsid_str) else {
            continue;
        };

        let Some(dll_path) = get_inproc_server32(&clsid) else {
            continue;
        };
        let dll_lower = dll_path.to_lowercase();
        let looks_like_tip = dll_lower.contains("ime")
            || dll_lower.contains("tip")
            || dll_lower.contains("textinput")
            || dll_lower.contains("input");
        if !looks_like_tip {
            continue;
        }

        let description = get_clsid_description(&clsid).unwrap_or_else(|| clsid_str.clone());
        let mut tip = TipInfo {
            clsid,
            description,
            dll_path,
            lang_id: 0x0411,
            profile_guid: GUID::zeroed(),
        };
        apply_known_tip_profile_fallback(&mut tip);
        tips.push(tip);
    }

    RegCloseKey(hkey_clsid).ok().ok();
    tips
}

/// Enumerate all registered TSF TIPs from the registry.
pub unsafe fn discover_tips() -> Vec<TipInfo> {
    let mut tips = Vec::new();
    let ctf_key_wide: Vec<u16> = CTF_TIP_KEY
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut hkey_ctf: HKEY = HKEY::default();

    if RegOpenKeyExW(
        HKEY_LOCAL_MACHINE,
        windows::core::PCWSTR(ctf_key_wide.as_ptr()),
        0,
        KEY_READ,
        &mut hkey_ctf,
    )
    .0 != 0
    {
        tracing::warn!("Failed to open CTF\\TIP registry key");
        return tips;
    }

    // Enumerate CLSID subkeys
    for clsid_idx in 0.. {
        let mut clsid_name = [0u16; 128];
        let mut clsid_name_len: u32 = clsid_name.len() as u32;

        let result = RegEnumKeyExW(
            hkey_ctf,
            clsid_idx,
            PWSTR(clsid_name.as_mut_ptr()),
            &mut clsid_name_len,
            None,
            PWSTR::null(),
            None,
            None,
        );

        if result != ERROR_SUCCESS {
            break;
        }

        let clsid_str = String::from_utf16_lossy(&clsid_name[..clsid_name_len as usize]);
        let clsid = match parse_guid(&clsid_str) {
            Some(g) => g,
            None => continue,
        };

        // Open LanguageProfile subkey
        let lp_path = format!("{}\\{}\\LanguageProfile", CTF_TIP_KEY, clsid_str);
        let lp_wide: Vec<u16> = lp_path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut hkey_lp: HKEY = HKEY::default();

        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            windows::core::PCWSTR(lp_wide.as_ptr()),
            0,
            KEY_READ,
            &mut hkey_lp,
        )
        .0 != 0
        {
            continue;
        }

        // Enumerate lang_id subkeys (e.g. "0x00000804")
        for lang_idx in 0.. {
            let mut lang_name = [0u16; 64];
            let mut lang_name_len: u32 = lang_name.len() as u32;

            if RegEnumKeyExW(
                hkey_lp,
                lang_idx,
                PWSTR(lang_name.as_mut_ptr()),
                &mut lang_name_len,
                None,
                PWSTR::null(),
                None,
                None,
            ) != ERROR_SUCCESS
            {
                break;
            }

            let lang_str = String::from_utf16_lossy(&lang_name[..lang_name_len as usize]);
            let lang_id = if lang_str.starts_with("0x") || lang_str.starts_with("0X") {
                u16::from_str_radix(&lang_str[2..], 16).unwrap_or(0)
            } else {
                lang_str.parse::<u16>().unwrap_or(0)
            };

            // Open profile GUID subkey
            let prof_path = format!("{}\\{}", lp_path, lang_str);
            let prof_wide: Vec<u16> = prof_path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut hkey_prof: HKEY = HKEY::default();

            if RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                windows::core::PCWSTR(prof_wide.as_ptr()),
                0,
                KEY_READ,
                &mut hkey_prof,
            )
            .0 != 0
            {
                continue;
            }

            // Enumerate profile GUID subkeys
            for guid_idx in 0.. {
                let mut guid_name = [0u16; 128];
                let mut guid_name_len: u32 = guid_name.len() as u32;

                if RegEnumKeyExW(
                    hkey_prof,
                    guid_idx,
                    PWSTR(guid_name.as_mut_ptr()),
                    &mut guid_name_len,
                    None,
                    PWSTR::null(),
                    None,
                    None,
                ) != ERROR_SUCCESS
                {
                    break;
                }

                let profile_str = String::from_utf16_lossy(&guid_name[..guid_name_len as usize]);
                let profile_guid = match parse_guid(&profile_str) {
                    Some(g) => g,
                    None => continue,
                };

                // Open this specific profile to read Description
                let desc_path = format!("{}\\{}", prof_path, profile_str);
                let desc_wide: Vec<u16> =
                    desc_path.encode_utf16().chain(std::iter::once(0)).collect();
                let mut hkey_desc: HKEY = HKEY::default();

                if RegOpenKeyExW(
                    HKEY_LOCAL_MACHINE,
                    windows::core::PCWSTR(desc_wide.as_ptr()),
                    0,
                    KEY_READ,
                    &mut hkey_desc,
                )
                .0 != 0
                {
                    continue;
                }

                let description =
                    read_reg_string(hkey_desc, "Description").unwrap_or_else(|| clsid_str.clone());

                RegCloseKey(hkey_desc).ok().ok();

                let dll_path = get_inproc_server32(&clsid).unwrap_or_else(|| String::new());

                let mut tip = TipInfo {
                    clsid,
                    description,
                    dll_path,
                    lang_id,
                    profile_guid,
                };
                apply_known_tip_profile_fallback(&mut tip);
                tips.push(tip);
            }

            RegCloseKey(hkey_prof).ok().ok();
        }

        RegCloseKey(hkey_lp).ok().ok();
    }

    RegCloseKey(hkey_ctf).ok().ok();

    let mut seen = std::collections::HashSet::new();
    for tip in &tips {
        seen.insert(tip.clsid);
    }
    for tip in discover_tip_clsid_fallbacks() {
        if seen.insert(tip.clsid) {
            tracing::warn!(
                "TIP CLSID {} present under Classes but missing from CTF\\\\TIP; using fallback discovery for {}",
                guid_to_registry_string(&tip.clsid),
                tip.description
            );
            tips.push(tip);
        }
    }
    tips
}

/// Resolve a TIP from user-specified identifiers.
/// At least one of clsid/name/dll must be provided.
pub unsafe fn resolve_tip(
    clsid: Option<&str>,
    name: Option<&str>,
    dll: Option<&str>,
) -> Result<TipInfo, String> {
    let explicit_dll = dll
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    // Direct CLSID lookup
    if let Some(clsid_str) = clsid {
        let guid =
            parse_guid(clsid_str).ok_or_else(|| format!("Invalid CLSID format: {}", clsid_str))?;

        // Look up DLL path and description from registry
        let dll_path = explicit_dll
            .clone()
            .or_else(|| get_inproc_server32(&guid))
            .unwrap_or_default();

        // Try to find description by scanning TIP registry
        let tips = discover_tips();
        if let Some(tip) = tips.iter().find(|t| t.clsid == guid) {
            let mut tip = tip.clone();
            if !dll_path.is_empty() {
                tip.dll_path = dll_path;
            }
            return Ok(tip);
        }

        // CLSID exists but not under CTF\TIP — create minimal info
        let mut tip = TipInfo {
            clsid: guid,
            description: clsid_str.to_string(),
            dll_path,
            lang_id: 0x0804, // generic fallback, may be overridden below
            profile_guid: GUID::zeroed(),
        };
        apply_known_tip_profile_fallback(&mut tip);
        return Ok(tip);
    }

    // Scan registry for name or DLL match
    let tips = discover_tips();
    if tips.is_empty() {
        return Err("No TSF TIPs found in registry".into());
    }

    if let Some(name_substr) = name {
        let name_lower = name_substr.to_lowercase();
        let matches: Vec<&TipInfo> = tips
            .iter()
            .filter(|t| t.description.to_lowercase().contains(&name_lower))
            .collect();

        if matches.len() == 1 {
            return Ok(matches[0].clone());
        } else if matches.is_empty() {
            return Err(format!(
                "No TIP found matching name '{}'. Available: {:?}",
                name_substr,
                tips.iter().map(|t| &t.description).collect::<Vec<_>>()
            ));
        } else {
            return Err(format!(
                "Multiple TIPs match name '{}': {:?}",
                name_substr,
                matches.iter().map(|t| &t.description).collect::<Vec<_>>()
            ));
        }
    }

    if let Some(dll_substr) = dll {
        let dll_lower = dll_substr.to_lowercase();
        let matches: Vec<&TipInfo> = tips
            .iter()
            .filter(|t| {
                t.dll_path.to_lowercase().contains(&dll_lower)
                    || path_file_name(&t.dll_path)
                        .map(|f| f.to_lowercase().contains(&dll_lower))
                        .unwrap_or(false)
            })
            .collect();

        if matches.len() == 1 {
            let mut tip = matches[0].clone();
            tip.dll_path = dll_substr.to_string();
            return Ok(tip);
        } else if matches.is_empty() {
            if let Some(clsid_str) = clsid {
                let guid = parse_guid(clsid_str)
                    .ok_or_else(|| format!("Invalid CLSID format: {}", clsid_str))?;
                let mut tip = TipInfo {
                    clsid: guid,
                    description: clsid_str.to_string(),
                    dll_path: dll_substr.to_string(),
                    lang_id: 0x0804,
                    profile_guid: GUID::zeroed(),
                };
                apply_known_tip_profile_fallback(&mut tip);
                return Ok(tip);
            }
            return Err(format!(
                "No TIP found matching DLL '{}'. Available: {:?}",
                dll_substr,
                tips.iter().map(|t| &t.dll_path).collect::<Vec<_>>()
            ));
        } else {
            return Err(format!(
                "Multiple TIPs match DLL '{}': {:?}",
                dll_substr,
                matches.iter().map(|t| &t.dll_path).collect::<Vec<_>>()
            ));
        }
    }

    Err("At least one of --tip-clsid, --tip-name, or --tip-dll must be provided".into())
}

fn path_file_name(path: &str) -> Option<String> {
    path.split(|c| c == '/' || c == '\\')
        .filter(|s| !s.is_empty())
        .last()
        .map(|s| s.to_string())
}
