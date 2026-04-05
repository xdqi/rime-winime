use windows::Win32::UI::Input::KeyboardAndMouse::{VkKeyScanW, MapVirtualKeyW, MAPVK_VK_TO_VSC};

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
        c if c < 0x7F => {
            unsafe { (VkKeyScanW(c as u16) & 0xFF) as u32 }
        }
        _ => keycode, // fallback
    }
}

pub fn make_l_key_data(vkey: u32, is_keyup: bool) -> u32 {
    let scan_code = unsafe { MapVirtualKeyW(vkey, MAPVK_VK_TO_VSC) };
    let repeat_count = 1u32;
    let is_extended = 0u32;
    let prev_key_state = if is_keyup { 1u32 } else { 0u32 };
    let transition_state = if is_keyup { 1u32 } else { 0u32 };

    (repeat_count & 0xFFFF)
        | ((scan_code & 0xFF) << 16)
        | ((is_extended & 1) << 24)
        | ((prev_key_state & 1) << 30)
        | ((transition_state & 1) << 31)
}

#[cfg(not(windows))]
pub fn rime_to_vk(keycode: u32) -> u32 { keycode }

#[cfg(not(windows))]
pub fn make_l_key_data(vkey: u32, is_keyup: bool) -> u32 { 0 }
