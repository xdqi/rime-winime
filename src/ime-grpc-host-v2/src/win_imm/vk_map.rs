use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_0, VK_1, VK_2, VK_3, VK_4, VK_5,
    VK_6, VK_7, VK_8, VK_9, VK_BACK, VK_CAPITAL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
    VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_NEXT, VK_OEM_1, VK_OEM_2,
    VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_CLEAR, VK_OEM_COMMA, VK_OEM_MINUS,
    VK_OEM_PERIOD, VK_OEM_PLUS, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT,
    VK_RWIN, VK_SPACE, VK_TAB, VK_UP,
};

pub fn is_shifted_char(keycode: u32) -> bool {
    match keycode {
        // Shifted numbers and punctuation
        0x21 | 0x40 | 0x23 | 0x24 | 0x25 | 0x5E | 0x26 | 0x2A | 0x28 | 0x29 | 0x7E | 0x5F
        | 0x2B | 0x7B | 0x7D | 0x7C | 0x3A | 0x22 | 0x3C | 0x3E | 0x3F => true,
        c if c >= 0x41 && c <= 0x5A => true, // A-Z
        _ => false,
    }
}

pub fn rime_to_vk(keycode: u32) -> VIRTUAL_KEY {
    match keycode {
        0xFF08 => VK_BACK,
        0xFF09 => VK_TAB,
        0xFF0D => VK_RETURN,
        0xFF1B => VK_ESCAPE,
        0x020 => VK_SPACE,
        0xFF50 => VK_HOME,
        0xFF51 => VK_LEFT,
        0xFF52 => VK_UP,
        0xFF53 => VK_RIGHT,
        0xFF54 => VK_DOWN,
        0xFF55 => VK_PRIOR,
        0xFF56 => VK_NEXT,
        0xFF57 => VK_END,
        0xFF63 => VK_INSERT,
        0xFFFF => VK_DELETE,

        // Modifier keys:
        0xFFE1 => VK_LSHIFT,
        0xFFE2 => VK_RSHIFT,
        0xFFE3 => VK_LCONTROL,
        0xFFE4 => VK_RCONTROL,
        0xFFE9 => VK_LMENU,
        0xFFEA => VK_RMENU,
        0xFFE5 => VK_CAPITAL,
        0xFFEB => VK_LWIN,
        0xFFEC => VK_RWIN,
        0xFF67 => VK_OEM_CLEAR,

        // Punctuation mapped explicitly to OEM virtual keys
        0x3B | 0x3A => VK_OEM_1,      // ; :
        0x3D | 0x2B => VK_OEM_PLUS,   // = +
        0x2C | 0x3C => VK_OEM_COMMA,  // , <
        0x2D | 0x5F => VK_OEM_MINUS,  // - _
        0x2E | 0x3E => VK_OEM_PERIOD, // . >
        0x2F | 0x3F => VK_OEM_2,      // / ?
        0x60 | 0x7E => VK_OEM_3,      // ` ~
        0x5B | 0x7B => VK_OEM_4,      // [ {
        0x5C | 0x7C => VK_OEM_5,      // \ |
        0x5D | 0x7D => VK_OEM_6,      // ] }
        0x27 | 0x22 => VK_OEM_7,      // ' "

        // Shifted numbers !@#$%^&*()
        0x21 => VK_1,
        0x40 => VK_2,
        0x23 => VK_3,
        0x24 => VK_4,
        0x25 => VK_5,
        0x5E => VK_6,
        0x26 => VK_7,
        0x2A => VK_8,
        0x28 => VK_9,
        0x29 => VK_0,

        c if c >= 0x41 && c <= 0x5A => {
            VIRTUAL_KEY(c as u16) // A-Z
        }
        c if c >= 0x61 && c <= 0x7A => {
            VIRTUAL_KEY((c - 0x20) as u16) // a-z maps to VK_A-VK_Z
        }
        c if c >= 0x30 && c <= 0x39 => {
            VIRTUAL_KEY(c as u16) // 0-9
        }

        // Punctuation (shifted versions) handled explicitly above, others fallback to keycode
        c if c < 0x7F => unsafe { VIRTUAL_KEY((VkKeyScanW(c as u16) & 0xFF) as u16) },
        _ => VIRTUAL_KEY(keycode as u16), // fallback
    }
}

pub fn make_l_key_data(vk: VIRTUAL_KEY, is_keyup: bool, is_alt: bool) -> u32 {
    let scan_code = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) };
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
pub fn rime_to_vk(keycode: u32) -> VIRTUAL_KEY {
    VIRTUAL_KEY(keycode as u16)
}

#[cfg(not(windows))]
pub fn make_l_key_data(vk: VIRTUAL_KEY, is_keyup: bool, is_alt: bool) -> u32 {
    0
}
