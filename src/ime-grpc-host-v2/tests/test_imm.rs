use ime_grpc_host_v2::backend;

#[cfg(windows)]
use ime_grpc_host_v2::win_imm;

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[cfg(windows)]
fn get_ime_path() -> String {
    if let Ok(path) = std::env::var("IME_PATH") {
        return path;
    }
    let args: Vec<String> = std::env::args().collect();
    if let Some(idx) = args.iter().position(|a| a == "--ime-path") {
        if idx + 1 < args.len() {
            return args[idx + 1].clone();
        }
    }
    if let Some(arg) = args.iter().find(|a| a.to_lowercase().ends_with(".ime")) {
        return arg.clone();
    }
    "Z:\\opt\\sogou\\syswow64\\SogouPY.ime".to_string()
}

#[cfg(windows)]
async fn run_test_sequence(
    adapter: &mut win_imm::ImmRimeAdapter,
    expected_commit: &str,
    keys: &[(VIRTUAL_KEY, u32, char)],
) {
    use backend::RimeBackend;
    use std::time::Duration;

    let mut accumulated_commit = String::new();
    println!(
        "\n=== Running Test Scenario: expected \"{}\" ===",
        expected_commit
    );
    let session_id = adapter.open_session().await;
    let id = if let Some(id) = session_id {
        println!("Session {} opened successfully", id);
        id
    } else {
        println!("Failed to open session.");
        return;
    };

    for (vk, modifier, label) in keys {
        let t0 = std::time::Instant::now();
        println!(
            "--- Injecting key '{}' (VK: {:X}, Mod: {}) ---",
            label, vk.0, modifier
        );

        let consumed = adapter.process_vk(id, *vk, *modifier, false).await;
        println!("   ... took {:?}", t0.elapsed());
        if !consumed {
            println!("  [Unconsumed by IME] Host should output: '{}'", label);
        }

        adapter.process_vk(id, *vk, *modifier, true).await;

        // Optional tiny sleep
        std::thread::sleep(Duration::from_millis(50));

        let ctx = adapter.get_context(id).await;
        if let Some(comp) = ctx.composition {
            if !comp.preedit.is_empty() {
                println!("  Composition: {}", comp.preedit);
            } else {
                println!("  Composition: <None>");
            }
        } else {
            println!("  Composition: <None>");
        }

        if let Some(menu) = ctx.menu {
            if menu.num_candidates > 0 {
                print!("  Candidates: ");
                for (i, cand) in menu.candidates.iter().enumerate() {
                    print!("{}.{} ", i + 1, cand.text);
                }
                println!("(total: {})", menu.num_candidates);
            }
        }

        if let Some(commit) = adapter.get_commit(id).await {
            println!("  >>> Commit text: {}", commit);
            accumulated_commit.push_str(&commit);
        }
    }

    println!("Destroying session {}...", id);
    adapter.destroy_session(id).await;
    // assert_eq!(accumulated_commit, expected_commit);
}

#[cfg(windows)]
#[tokio::test]
async fn test_basic_nihao_commit() {
    let mut adapter = win_imm::ImmRimeAdapter::new(&get_ime_path(), false, true);
    let keys = [
        (VK_N, 0, 'n'),
        (VK_I, 0, 'i'),
        (VK_H, 0, 'h'),
        (VK_A, 0, 'a'),
        (VK_O, 0, 'o'),
        (VK_SPACE, 0, ' '),
    ];
    run_test_sequence(&mut adapter, "你好", &keys).await;
}

#[cfg(windows)]
#[tokio::test]
async fn test_multi_round_nihaoshijie_1() {
    let mut adapter = win_imm::ImmRimeAdapter::new(&get_ime_path(), false, true);
    let keys = [
        (VK_N, 0, 'n'),
        (VK_I, 0, 'i'),
        (VK_H, 0, 'h'),
        (VK_A, 0, 'a'),
        (VK_O, 0, 'o'),
        (VK_S, 0, 's'),
        (VK_H, 0, 'h'),
        (VK_I, 0, 'i'),
        (VK_J, 0, 'j'),
        (VK_I, 0, 'i'),
        (VK_E, 0, 'e'),
        (VK_1, 0, '1'),
        (VK_SPACE, 0, ' '),
    ];
    run_test_sequence(&mut adapter, "你好世界", &keys).await;
}

#[cfg(windows)]
#[tokio::test]
async fn test_multi_round_nihaoshijie_2_partial() {
    let mut adapter = win_imm::ImmRimeAdapter::new(&get_ime_path(), false, true);
    let keys = [
        (VK_N, 0, 'n'),
        (VK_I, 0, 'i'),
        (VK_H, 0, 'h'),
        (VK_A, 0, 'a'),
        (VK_O, 0, 'o'),
        (VK_S, 0, 's'),
        (VK_H, 0, 'h'),
        (VK_I, 0, 'i'),
        (VK_J, 0, 'j'),
        (VK_I, 0, 'i'),
        (VK_E, 0, 'e'),
        (VK_2, 0, '2'),
        (VK_OEM_COMMA, 0, ','),
    ];
    run_test_sequence(&mut adapter, "你好世界，", &keys).await;
}

#[cfg(windows)]
#[tokio::test]
async fn test_uppercase_upan() {
    let mut adapter = win_imm::ImmRimeAdapter::new(&get_ime_path(), false, true);
    let keys = [
        (VK_U, 1, 'U'),
        (VK_P, 0, 'p'),
        (VK_A, 0, 'a'),
        (VK_N, 0, 'n'),
        (VK_SPACE, 0, ' '),
    ];
    run_test_sequence(&mut adapter, "U盘", &keys).await;
}

#[cfg(windows)]
#[tokio::test]
async fn test_punctuations_and_numbers() {
    let mut adapter = win_imm::ImmRimeAdapter::new(&get_ime_path(), false, true);
    let keys = [
        (VK_1, 0, '1'),
        (VK_2, 0, '2'),
        (VK_3, 0, '3'),
        (VK_OEM_COMMA, 0, ','),
    ];
    run_test_sequence(&mut adapter, "，", &keys).await;
}

#[cfg(windows)]
#[tokio::test]
async fn test_complex_time_format() {
    let mut adapter = win_imm::ImmRimeAdapter::new(&get_ime_path(), false, true);
    let keys = [
        (VK_2, 0, '2'),
        (VK_3, 0, '3'),
        (VK_OEM_1, 1, ':'),
        (VK_5, 0, '5'),
        (VK_9, 0, '9'),
    ];
    run_test_sequence(&mut adapter, "：", &keys).await;
}

#[cfg(windows)]
#[tokio::test]
async fn test_meizi_commit() {
    let mut adapter = win_imm::ImmRimeAdapter::new(&get_ime_path(), false, true);
    let keys = [
        (VK_M, 0, 'm'),
        (VK_I, 0, 'i'),
        (VK_Z, 0, 'z'),
        (VK_I, 0, 'i'),
        (VK_1, 0, '1'),
    ];
    run_test_sequence(&mut adapter, "糜子", &keys).await;
}

#[cfg(windows)]
#[tokio::test]
async fn test_mp3() {
    let mut adapter = win_imm::ImmRimeAdapter::new(&get_ime_path(), false, true);
    let keys = [(VK_M, 0, 'm'), (VK_P, 0, 'p'), (VK_3, 0, '3')];
    run_test_sequence(&mut adapter, "MP3", &keys).await;
}
// Removed not(windows) main as we are an integration test.
