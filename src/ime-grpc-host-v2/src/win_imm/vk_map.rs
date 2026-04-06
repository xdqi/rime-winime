use windows::Win32::UI::Input::KeyboardAndMouse::{VkKeyScanW, MapVirtualKeyW, MAPVK_VK_TO_VSC};

pub fn is_shifted_char(keycode: u32) -> bool {
    match keycode {
        // Shifted numbers and punctuation
        0x21 | 0x40 | 0x23 | 0x24 | 0x25 | 0x5E | 0x26 | 0x2A | 0x28 | 0x29 |
        0x7E | 0x5F | 0x2B | 0x7B | 0x7D | 0x7C | 0x3A | 0x22 | 0x3C | 0x3E | 0x3F => true,
        c if c >= 0x41 && c <= 0x5A => true, // A-Z
        _ => false,
    }
}

pub fn rime_to_vk(keycode: u32) -> u32 {
    match keycode {
        0xFF08 => 0x08, // VK_BACK
        0xFF09 => 0x09, // VK_TAB
        0xFF0D => 0x0D, // VK_RETURN
        0xFF1B => 0x1B, // VK_ESCAPE
        0x020  => 0x20, // VK_SPACE
        0xFF50 => 0x24, // VK_HOME
        0xFF51 => 0x25, // VK_LEFT
        0xFF52 => 0x26, // VK_UP
        0xFF53 => 0x27, // VK_RIGHT
        0xFF54 => 0x28, // VK_DOWN
        0xFF55 => 0x21, // VK_PRIOR (PAGEUP)
        0xFF56 => 0x22, // VK_NEXT (PAGEDOWN)
        0xFF57 => 0x23, // VK_END
        0xFF63 => 0x2D, // VK_INSERT
        0xFFFF => 0x2E, // VK_DELETE
        
        // Modifier keys:
        0xFFE1 => 0xA0, // VK_LSHIFT
        0xFFE2 => 0xA1, // VK_RSHIFT
        0xFFE3 => 0xA2, // VK_LCONTROL
        0xFFE4 => 0xA3, // VK_RCONTROL
        0xFFE9 => 0xA4, // VK_LMENU
        0xFFEA => 0xA5, // VK_RMENU
        0xFFE5 => 0x14, // VK_CAPITAL (Caps Lock)
        0xFFEB => 0x5B, // VK_LWIN
        0xFFEC => 0x5C, // VK_RWIN
        0xFF67 => 0x93, // VK_OEM_CLEAR (Menu)

        // Punctuation mapped explicitly to OEM virtual keys
        0x3B | 0x3A => 0xBA, // VK_OEM_1 (;) and (:)
        0x3D | 0x2B => 0xBB, // VK_OEM_PLUS (=) and (+)
        0x2C | 0x3C => 0xBC, // VK_OEM_COMMA (,) and (<)
        0x2D | 0x5F => 0xBD, // VK_OEM_MINUS (-) and (_)
        0x2E | 0x3E => 0xBE, // VK_OEM_PERIOD (.) and (>)
        0x2F | 0x3F => 0xBF, // VK_OEM_2 (/) and (?)
        0x60 | 0x7E => 0xC0, // VK_OEM_3 (`) and (~)
        0x5B | 0x7B => 0xDB, // VK_OEM_4 ([) and ({)
        0x5C | 0x7C => 0xDC, // VK_OEM_5 (\) and (|)
        0x5D | 0x7D => 0xDD, // VK_OEM_6 (]) and (})
        0x27 | 0x22 => 0xDE, // VK_OEM_7 (') and (")

        // Shifted numbers !@#$%^&*()
        0x21 => 0x31,
        0x40 => 0x32,
        0x23 => 0x33,
        0x24 => 0x34,
        0x25 => 0x35,
        0x5E => 0x36,
        0x26 => 0x37,
        0x2A => 0x38,
        0x28 => 0x39,
        0x29 => 0x30,

        c if c >= 0x41 && c <= 0x5A => {
            c // A-Z
        }
        c if c >= 0x61 && c <= 0x7A => {
            c - 0x20 // a-z maps to 0x41-0x5A
        }
        c if c >= 0x30 && c <= 0x39 => {
            c // 0-9
        }
        
        // Punctuation (shifted versions) handled explicitly above, others fallback to keycode
        
        c if c < 0x7F => {
            unsafe { (VkKeyScanW(c as u16) & 0xFF) as u32 }
        }
        _ => keycode, // fallback
    }
}

pub fn make_l_key_data(vkey: u32, is_keyup: bool, is_alt: bool) -> u32 {
    let scan_code = unsafe { MapVirtualKeyW(vkey, MAPVK_VK_TO_VSC) };
    let repeat_count = 1u32;
    let is_extended = 0u32;
    let prev_key_state = if is_keyup { 1u32 } else { 0u32 };
    let transition_state = if is_keyup { 1u32 } else { 0u32 };
    let context_code = if is_alt { 1u32 } else { 0u32 }; // Bit 29 for ALT key flag

    (repeat_count & 0xFFFF)
        | ((scan_code & 0xFF) << 16)
        | ((is_extended & 1) << 24)
        | ((context_code & 1) << 29)
        | ((prev_key_state & 1) << 30)
        | ((transition_state & 1) << 31)
}

#[cfg(not(windows))]
pub fn rime_to_vk(keycode: u32) -> u32 { keycode }

#[cfg(not(windows))]
pub fn make_l_key_data(vkey: u32, is_keyup: bool, is_alt: bool) -> u32 { 0 }
