import re

with open("/opt/sogou/src/ime-grpc-host/src/backend/win_imm_full.rs", "r") as f:
    content = f.read()

# Remove exact line "#[cfg(windows)]\n"
content = re.sub(r'#\[cfg\(windows\)\]\n', '', content)
# Sometimes it's indented
content = re.sub(r'[ \t]*#\[cfg\(windows\)\]\n', '', content)

# Remove not(windows) blocks
# Unfortunately these are blocks:
#         #[cfg(not(windows))]
#         {
#            ...
#         }
# For this file, the blocks are small and well-known. Let's just rely on rustc to ignore them if we keep them and they are unconditionally compiled?
# Wait! If we remove #[cfg(not(windows))], then those blocks become UNCONDITIONAL!
# No, we only want to keep the windows blocks! So we shouldn't strip #[cfg(not(windows))] - wait, they won't be compiled anyway inside `mod imp` because `mod imp` is ONLY compiled when `#[cfg(windows)]`. Those not(windows) blocks inside `imp` will be dead code ignored by rustc because the whole `imp` module is `cfg(windows)`.
# Even better: if they stay, they are practically just no-ops, but their contents might cause compilation error if types don't match, etc. Wait, if the whole file is `cfg(windows)`, any `cfg(not(windows))` inside is trivially false. Rustc evaluates `cfg` macro anywhere! 
# So if I keep `#[cfg(not(windows))]`, rustc will successfully IGNORE that block. No issue!

with open("/opt/sogou/src/ime-grpc-host/src/backend/imp.rs", "w") as f:
    f.write(content)

