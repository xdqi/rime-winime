use std::thread;
use std::time::Duration;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE};
use windows::core::w;

fn main() {
    println!("Main thread sleeping...");
    thread::sleep(Duration::from_secs(2));
    let handle = thread::spawn(|| unsafe {
        println!("Background thread creating window...");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("HiddenImeWindow"),
            WINDOW_STYLE(0),
            0, 0, 0, 0,
            HWND_MESSAGE,
            None,
            None,
            None,
        );
        match hwnd {
            Ok(h) => println!("Created window: {:?}", h),
            Err(e) => println!("Failed to create window: {:?}", e),
        }
    });
    handle.join().unwrap();
}
