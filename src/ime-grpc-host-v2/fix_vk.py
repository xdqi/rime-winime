mappings = {
    ';': 0xBA,
    ':': 0xBA,
    '=': 0xBB,
    '+': 0xBB,
    ',': 0xBC,
    '<': 0xBC,
    '-': 0xBD,
    '_': 0xBD,
    '.': 0xBE,
    '>': 0xBE,
    '/': 0xBF,
    '?': 0xBF,
    '`': 0xC0,
    '~': 0xC0,
    '[': 0xDB,
    '{': 0xDB,
    '\\': 0xDC,
    '|': 0xDC,
    ']': 0xDD,
    '}': 0xDD,
    '\'': 0xDE,
    '"': 0xDE,
}
print("    match keycode {")
prev = None
for k, v in mappings.items():
    print(f"        0x{ord(k):02X} => 0x{v:02X},")
print("    }")
