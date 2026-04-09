//! Reproduce sub_1801BECD0's version detection logic:
//! 1) kernel32.dll PE file version (GetFileVersionInfoW + VerQueryValueW)
//! 2) RtlGetNtVersionNumbers
//! Then sub_1801B6D70 checks: if major > 6 || (major == 6 && minor >= 2) → byte_3554 = 1
use std::ffi::c_void;

#[link(name = "version")]
extern "system" {
    fn GetFileVersionInfoSizeW(lptstr_filename: *const u16, lpdw_handle: *mut u32) -> u32;
    fn GetFileVersionInfoW(
        lptstr_filename: *const u16,
        dw_handle: u32,
        dw_len: u32,
        lp_data: *mut c_void,
    ) -> i32;
    fn VerQueryValueW(
        p_block: *const c_void,
        lp_sub_block: *const u16,
        lplp_buffer: *mut *mut c_void,
        pu_len: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemDirectoryW(lp_buffer: *mut u16, u_size: u32) -> u32;
    fn GetModuleHandleW(lp_module_name: *const u16) -> *mut c_void;
    fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *mut c_void;
    fn GetVersionExW(lp_version_information: *mut OSVERSIONINFOW) -> i32;
}

#[repr(C)]
struct VS_FIXEDFILEINFO {
    dw_signature: u32,
    dw_struc_version: u32,
    dw_file_version_ms: u32,
    dw_file_version_ls: u32,
    dw_product_version_ms: u32,
    dw_product_version_ls: u32,
    dw_file_flags_mask: u32,
    dw_file_flags: u32,
    dw_file_os: u32,
    dw_file_type: u32,
    dw_file_subtype: u32,
    dw_file_date_ms: u32,
    dw_file_date_ls: u32,
}

#[repr(C)]
struct OSVERSIONINFOW {
    dw_os_version_info_size: u32,
    dw_major_version: u32,
    dw_minor_version: u32,
    dw_build_number: u32,
    dw_platform_id: u32,
    sz_csd_version: [u16; 128],
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[test]
fn test_version_detection() {
    unsafe {
        println!("\n=== SogouPY version detection reproduction ===\n");

        // --- Method 1: kernel32.dll PE file version ---
        let mut sys_dir = vec![0u16; 260];
        let len = GetSystemDirectoryW(sys_dir.as_mut_ptr(), 260);
        let sys_dir_str = String::from_utf16_lossy(&sys_dir[..len as usize]);
        let kernel32_path = format!("{}\\kernel32.dll", sys_dir_str);
        let kernel32_wide = to_wide(&kernel32_path);
        println!("kernel32.dll path: {}", kernel32_path);

        let mut handle: u32 = 0;
        let size = GetFileVersionInfoSizeW(kernel32_wide.as_ptr(), &mut handle);
        println!("GetFileVersionInfoSizeW: size={}, handle={}", size, handle);

        if size > 0 {
            let mut buf = vec![0u8; size as usize];
            let ok = GetFileVersionInfoW(
                kernel32_wide.as_ptr(),
                0,
                size,
                buf.as_mut_ptr() as *mut c_void,
            );
            println!("GetFileVersionInfoW: ok={}", ok);

            if ok != 0 {
                let sub_block = to_wide("\\");
                let mut lp_buffer: *mut c_void = std::ptr::null_mut();
                let mut pu_len: u32 = 0;
                let ok2 = VerQueryValueW(
                    buf.as_ptr() as *const c_void,
                    sub_block.as_ptr(),
                    &mut lp_buffer,
                    &mut pu_len,
                );
                println!("VerQueryValueW: ok={}, len={}", ok2, pu_len);

                if ok2 != 0 && !lp_buffer.is_null() {
                    let ffi = &*(lp_buffer as *const VS_FIXEDFILEINFO);
                    let file_major = (ffi.dw_file_version_ms >> 16) & 0xFFFF;
                    let file_minor = ffi.dw_file_version_ms & 0xFFFF;
                    let file_build = (ffi.dw_file_version_ls >> 16) & 0xFFFF;
                    let file_rev = ffi.dw_file_version_ls & 0xFFFF;
                    let prod_major = (ffi.dw_product_version_ms >> 16) & 0xFFFF;
                    let prod_minor = ffi.dw_product_version_ms & 0xFFFF;
                    let prod_build = (ffi.dw_product_version_ls >> 16) & 0xFFFF;
                    let prod_rev = ffi.dw_product_version_ls & 0xFFFF;
                    println!(
                        "  FileVersion:    {}.{}.{}.{}",
                        file_major, file_minor, file_build, file_rev
                    );
                    println!(
                        "  ProductVersion: {}.{}.{}.{}",
                        prod_major, prod_minor, prod_build, prod_rev
                    );

                    // SogouPY sub_1801BECD0 extracts:
                    // v105[1] = HIWORD(dwProductVersionMS) = prod_major (as u16)
                    // v105[2] = LOWORD(dwProductVersionMS) = prod_minor (via v96)
                    // v105[3] = HIWORD(dwProductVersionLS) = prod_build
                    // v103[1] = HIWORD(dwFileVersionLS) = file_build
                    // v103[2] = LOWORD(dwFileVersionLS) = file_rev (via v13)
                    // v103[3] = HIWORD(...)
                    // Then later picks max(kernel32 version, RtlGetNtVersionNumbers)
                    println!(
                        "\n  [kernel32.dll PE] version for SogouPY: major={}, minor={}",
                        prod_major, prod_minor
                    );

                    let is_win8_or_later = prod_major > 6 || (prod_major == 6 && prod_minor >= 2);
                    println!(
                        "  byte_3554 would be set? {} (major={} minor={})",
                        is_win8_or_later, prod_major, prod_minor
                    );
                }
            }
        } else {
            println!("  kernel32.dll version info NOT available");
        }

        // --- Method 2: RtlGetNtVersionNumbers ---
        println!("\n--- RtlGetNtVersionNumbers ---");
        let ntdll = GetModuleHandleW(to_wide("ntdll.dll").as_ptr());
        if !ntdll.is_null() {
            let proc = GetProcAddress(ntdll, b"RtlGetNtVersionNumbers\0".as_ptr());
            if !proc.is_null() {
                let func: extern "system" fn(*mut u32, *mut u32, *mut u32) =
                    std::mem::transmute(proc);
                let mut major: u32 = 0;
                let mut minor: u32 = 0;
                let mut build: u32 = 0;
                func(&mut major, &mut minor, &mut build);
                println!(
                    "  RtlGetNtVersionNumbers: {}.{}.{}",
                    major,
                    minor,
                    build & 0xFFFF
                );

                let is_win8_or_later = major > 6 || (major == 6 && minor >= 2);
                println!(
                    "  byte_3554 would be set? {} (major={} minor={})",
                    is_win8_or_later, major, minor
                );
            } else {
                println!("  RtlGetNtVersionNumbers not found");
            }
        }

        // --- Method 3: GetVersionExW (compatibility API, affected by manifest/registry) ---
        println!("\n--- GetVersionExW (compat) ---");
        let mut osvi: OSVERSIONINFOW = std::mem::zeroed();
        osvi.dw_os_version_info_size = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        let ok = GetVersionExW(&mut osvi);
        if ok != 0 {
            println!(
                "  GetVersionExW: {}.{}.{}",
                osvi.dw_major_version, osvi.dw_minor_version, osvi.dw_build_number
            );
            let is_win8_or_later = osvi.dw_major_version > 6
                || (osvi.dw_major_version == 6 && osvi.dw_minor_version >= 2);
            println!("  byte_3554 would be set? {}", is_win8_or_later);
        }

        // --- Method 4: RtlGetVersion (not affected by compat shim) ---
        println!("\n--- RtlGetVersion (unshimmed) ---");
        if !ntdll.is_null() {
            let proc = GetProcAddress(ntdll, b"RtlGetVersion\0".as_ptr());
            if !proc.is_null() {
                let func: extern "system" fn(*mut OSVERSIONINFOW) -> i32 =
                    std::mem::transmute(proc);
                let mut osvi2: OSVERSIONINFOW = std::mem::zeroed();
                osvi2.dw_os_version_info_size = std::mem::size_of::<OSVERSIONINFOW>() as u32;
                func(&mut osvi2);
                println!(
                    "  RtlGetVersion: {}.{}.{}",
                    osvi2.dw_major_version, osvi2.dw_minor_version, osvi2.dw_build_number
                );
            }
        }

        println!("\n=== Summary ===");
        println!("SogouPY uses max(kernel32.dll PE version, RtlGetNtVersionNumbers).");
        println!("If that max >= 6.2, byte_3554=1 → uses SendMessageW(0x8BB8) instead of ImmGenerateMessage.");
        println!("Wine's 'winecfg /v win7' only affects GetVersionExW, NOT the PE version or RtlGetNtVersionNumbers.");
    }
}
