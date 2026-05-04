use xkeysym::key as xk;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_0, VK_1, VK_2, VK_3, VK_4, VK_5,
    VK_6, VK_7, VK_8, VK_9, VK_BACK, VK_CAPITAL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
    VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_NEXT, VK_OEM_1, VK_OEM_2,
    VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_CLEAR, VK_OEM_COMMA, VK_OEM_MINUS,
    VK_OEM_PERIOD, VK_OEM_PLUS, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT,
    VK_RWIN, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{KF_ALTDOWN, KF_REPEAT, KF_UP};

pub fn is_shifted_char(keycode: u32) -> bool {
    match keycode {
        xk::exclam
        | xk::at
        | xk::numbersign
        | xk::dollar
        | xk::percent
        | xk::asciicircum
        | xk::ampersand
        | xk::asterisk
        | xk::parenleft
        | xk::parenright
        | xk::asciitilde
        | xk::underscore
        | xk::plus
        | xk::braceleft
        | xk::braceright
        | xk::bar
        | xk::colon
        | xk::quotedbl
        | xk::less
        | xk::greater
        | xk::question => true,
        c if (xk::A..=xk::Z).contains(&c) => true,
        _ => false,
    }
}

pub fn rime_to_vk(keycode: u32) -> VIRTUAL_KEY {
    match keycode {
        xk::BackSpace => VK_BACK,
        xk::Tab => VK_TAB,
        xk::Return => VK_RETURN,
        xk::Escape => VK_ESCAPE,
        xk::space => VK_SPACE,
        xk::Home => VK_HOME,
        xk::Left => VK_LEFT,
        xk::Up => VK_UP,
        xk::Right => VK_RIGHT,
        xk::Down => VK_DOWN,
        xk::Prior => VK_PRIOR,
        xk::Next => VK_NEXT,
        xk::End => VK_END,
        xk::Insert => VK_INSERT,
        xk::Delete => VK_DELETE,
        xk::Shift_L => VK_LSHIFT,
        xk::Shift_R => VK_RSHIFT,
        xk::Control_L => VK_LCONTROL,
        xk::Control_R => VK_RCONTROL,
        xk::Alt_L => VK_LMENU,
        xk::Alt_R => VK_RMENU,
        xk::Caps_Lock => VK_CAPITAL,
        xk::Super_L => VK_LWIN,
        xk::Super_R => VK_RWIN,
        xk::Menu => VK_OEM_CLEAR,
        xk::semicolon | xk::colon => VK_OEM_1,
        xk::equal | xk::plus => VK_OEM_PLUS,
        xk::comma | xk::less => VK_OEM_COMMA,
        xk::minus | xk::underscore => VK_OEM_MINUS,
        xk::period | xk::greater => VK_OEM_PERIOD,
        xk::slash | xk::question => VK_OEM_2,
        xk::grave | xk::asciitilde => VK_OEM_3,
        xk::bracketleft | xk::braceleft => VK_OEM_4,
        xk::backslash | xk::bar => VK_OEM_5,
        xk::bracketright | xk::braceright => VK_OEM_6,
        xk::apostrophe | xk::quotedbl => VK_OEM_7,
        xk::exclam => VK_1,
        xk::at => VK_2,
        xk::numbersign => VK_3,
        xk::dollar => VK_4,
        xk::percent => VK_5,
        xk::asciicircum => VK_6,
        xk::ampersand => VK_7,
        xk::asterisk => VK_8,
        xk::parenleft => VK_9,
        xk::parenright => VK_0,
        c if (xk::A..=xk::Z).contains(&c) => VIRTUAL_KEY(c as u16),
        c if (xk::a..=xk::z).contains(&c) => VIRTUAL_KEY((c - 0x20) as u16),
        c if (xk::_0..=xk::_9).contains(&c) => VIRTUAL_KEY(c as u16),
        c if c < 0x7F => unsafe { VIRTUAL_KEY((VkKeyScanW(c as u16) & 0xFF) as u16) },
        _ => VIRTUAL_KEY(keycode as u16),
    }
}

pub fn make_l_key_data(vk: VIRTUAL_KEY, is_keyup: bool, is_alt: bool) -> u32 {
    let scan_code = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) };
    let repeat_count = 1u32;
    let mut hi = scan_code & 0xFF;
    if is_keyup {
        hi |= KF_REPEAT | KF_UP;
    }
    if is_alt {
        hi |= KF_ALTDOWN;
    }

    (repeat_count & 0xFFFF) | (hi << 16)
}

#[cfg(not(windows))]
pub fn rime_to_vk(keycode: u32) -> VIRTUAL_KEY {
    VIRTUAL_KEY(keycode as u16)
}

#[cfg(not(windows))]
pub fn make_l_key_data(_vk: VIRTUAL_KEY, _is_keyup: bool, _is_alt: bool) -> u32 {
    0
}
