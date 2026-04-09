/// Chinese punctuation mapping for SogouPY.
///
/// When SogouPY returns WM_IME_COMPOSITION(GCS_RESULTSTR) but
/// COMPOSITIONSTRING is empty (standalone punctuation with no prior
/// composition), we fall back to this mapping to produce the expected
/// fullwidth / CJK punctuation character.
///
/// The mapping mirrors SogouPY's `OnInterpunction` logic:
///   1. Dictionary lookup (sub_180555790) for special cases
///   2. Fallback: char + 0xFEE0 (ASCII→fullwidth block)

/// Map an ASCII keycode (Rime keycode = ASCII code point) to the
/// corresponding Chinese punctuation string.  Returns `None` for
/// characters that SogouPY does not remap (letters, digits, etc.).
pub fn lookup(keycode: u32) -> Option<&'static str> {
    // SogouPY's dictionary-based mappings (override the mechanical +0xFEE0).
    // These match the default SogouPinyin punctuation table.
    match keycode as u8 as char {
        ',' => Some("\u{FF0C}"),         // ，
        '.' => Some("\u{3002}"),         // 。
        ';' => Some("\u{FF1B}"),         // ；
        ':' => Some("\u{FF1A}"),         // ：
        '?' => Some("\u{FF1F}"),         // ？
        '!' => Some("\u{FF01}"),         // ！
        '\\' => Some("\u{3001}"),        // 、
        '(' => Some("\u{FF08}"),         // （
        ')' => Some("\u{FF09}"),         // ）
        '<' => Some("\u{300A}"),         // 《
        '>' => Some("\u{300B}"),         // 》
        '[' => Some("\u{3010}"),         // 【
        ']' => Some("\u{3011}"),         // 】
        '{' => Some("\u{FF5B}"),         // ｛
        '}' => Some("\u{FF5D}"),         // ｝
        '^' => Some("\u{2026}\u{2026}"), // ……
        '_' => Some("\u{2014}\u{2014}"), // ——
        '~' => Some("\u{FF5E}"),         // ～
        '`' => Some("\u{00B7}"),         // ·
        '$' => Some("\u{FFE5}"),         // ￥
        ' ' => Some("\u{3000}"),         // ideographic space
        // For remaining printable ASCII 33–126, use the mechanical
        // fullwidth mapping (char + 0xFEE0).
        c if c.is_ascii_graphic() => None, // caller falls through to +0xFEE0
        _ => None,
    }
}

/// Map an ASCII keycode to a fullwidth character using the mechanical
/// `+0xFEE0` rule (SogouPY's fallback when the dictionary has no entry).
pub fn fullwidth_fallback(keycode: u32) -> Option<char> {
    let c = keycode as u8;
    if (33..=126).contains(&c) {
        // ASCII printable → Unicode fullwidth block
        char::from_u32(c as u32 + 0xFEE0)
    } else {
        None
    }
}

/// Combined lookup: dictionary first, then mechanical fallback.
pub fn map_punctuation(keycode: u32) -> Option<String> {
    if let Some(s) = lookup(keycode) {
        return Some(s.to_string());
    }
    fullwidth_fallback(keycode).map(|c| c.to_string())
}
