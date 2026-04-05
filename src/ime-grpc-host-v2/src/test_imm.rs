pub mod proto {
    pub mod rime_service_v2 {
        tonic::include_proto!("rime.service.v2");
    }
}

pub mod backend;
#[cfg(windows)]
pub mod win_imm;

#[cfg(windows)]
fn run_test_sequence(adapter: &mut win_imm::ImmRimeAdapter, name: &str, keys: &[(u32, u32, char)]) {
    use backend::RimeBackend;
    use proto::rime_service_v2::KeyEvent;
    use std::time::Duration;

    println!("\n=== Running Test Scenario: {} ===", name);
    let session_id = adapter.open_session();
    let id = if let Some(id) = session_id {
        println!("Session {} opened successfully", id);
        id
    } else {
        println!("Failed to open session.");
        return;
    };

    for (keycode, modifier, label) in keys {
        println!("--- Injecting key '{}' ({:X}) ---", label, keycode);
        let evt = KeyEvent {
            keycode: *keycode,
            modifier: *modifier,
        };
        
        let consumed = adapter.process_key(id, &evt);
        if !consumed {
            println!("  [Unconsumed by IME] Host should output: '{}'", label);
        }
        
        // Optional tiny sleep
        std::thread::sleep(Duration::from_millis(50));
        
        let ctx = adapter.get_context(id);
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
                for (i, cand) in menu.candidates.iter().take(3).enumerate() {
                    print!("{}.{} ", i+1, cand.text);
                }
                println!("(total: {})", menu.num_candidates);
            }
        }

        if let Some(commit) = adapter.get_commit(id) {
            println!("  >>> Commit text: {}", commit);
        }
    }

    println!("Destroying session {}...", id);
    adapter.destroy_session(id);
}

#[cfg(windows)]
fn main() {
    println!("Initializing ImmRimeAdapter...");
    let mut adapter = win_imm::ImmRimeAdapter::new();
    
    // Test 1: Simple "nihao"
    let keys_nihao = [
        (0x6E, 0, 'n'), // 'n' in ASCII
        (0x69, 0, 'i'), // 'i' in ASCII
        (0x68, 0, 'h'), // 'h'
        (0x61, 0, 'a'), // 'a'
        (0x6F, 0, 'o'), // 'o'
        (0x20, 0, ' '), // Space
    ];
    run_test_sequence(&mut adapter, "Basic 'nihao' commit", &keys_nihao);

    // Test 2: "nihaoshijie" and press '1'
    let keys_nihaoshijie_1 = [
        (0x6E, 0, 'n'),
        (0x69, 0, 'i'),
        (0x68, 0, 'h'),
        (0x61, 0, 'a'),
        (0x6F, 0, 'o'),
        (0x73, 0, 's'),
        (0x68, 0, 'h'),
        (0x69, 0, 'i'),
        (0x6A, 0, 'j'),
        (0x69, 0, 'i'),
        (0x65, 0, 'e'),
        (0x31, 0, '1'), // '1'
        (0x20, 0, ' '), // Space
    ];
    run_test_sequence(&mut adapter, "Multi-round 'nihaoshijie' + '1'", &keys_nihaoshijie_1);

    // Test 3: "nihaoshijie" and press '2' (Partial commit)
    let keys_nihaoshijie_2 = [
        (0x6E, 0, 'n'),
        (0x69, 0, 'i'),
        (0x68, 0, 'h'),
        (0x61, 0, 'a'),
        (0x6F, 0, 'o'),
        (0x73, 0, 's'),
        (0x68, 0, 'h'),
        (0x69, 0, 'i'),
        (0x6A, 0, 'j'),
        (0x69, 0, 'i'),
        (0x65, 0, 'e'),
        (0x32, 0, '2'), // '2'
        (0x2C, 0, ','), // ','
    ];
    run_test_sequence(&mut adapter, "Multi-round 'nihaoshijie' + '2' + ',' (Partial commit)", &keys_nihaoshijie_2);

    // Test 4: Uppercase letter start (e.g. 'Upan')
    let keys_upan = [
        (0x55, 1, 'U'), // Shift + U
        (0x70, 0, 'p'), // 'p'
        (0x61, 0, 'a'), // 'a'
        (0x6E, 0, 'n'), // 'n'
        (0x20, 0, ' '), // Space
    ];
    run_test_sequence(&mut adapter, "Uppercase 'Upan'", &keys_upan);

    // Test 5: Punctuations and numbers initially
    let keys_punct_num = [
        (0x31, 0, '1'),
        (0x32, 0, '2'),
        (0x33, 0, '3'),
        (0x2C, 0, ','), // ','
    ];
    run_test_sequence(&mut adapter, "Punctuations and Numbers '123,'", &keys_punct_num);

    // Test 6: Complex time format '23:59'
    let keys_time = [
        (0x32, 0, '2'),
        (0x33, 0, '3'),
        (0x3A, 1, ':'), // Shift + ':'
        (0x35, 0, '5'),
        (0x39, 0, '9'),
    ];
    run_test_sequence(&mut adapter, "Time format '23:59'", &keys_time);
}

#[cfg(not(windows))]
fn main() {
    println!("This test runs only on Windows targets.");
}
