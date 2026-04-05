pub mod proto {
    pub mod rime_service_v2 {
        tonic::include_proto!("rime.service.v2");
    }
}
#[path = "../backend/mod.rs"]
pub mod backend;
#[path = "../win_imm/mod.rs"]
pub mod win_imm;

use windows::Win32::UI::Input::Ime::{ImmNotifyIME, NI_SELECTCANDIDATESTR};

#[tokio::main]
async fn main() {
    let mut adapter = win_imm::ImmRimeAdapter::new();
    use backend::RimeBackend;
    let id = adapter.open_session().await.unwrap();
    use proto::rime_service_v2::KeyEvent;
    // Inject 'n' 'i'
    adapter.process_key(id, &KeyEvent { keycode: 0x6E, modifier: 0 }).await; // 'n'
    adapter.process_key(id, &KeyEvent { keycode: 0x69, modifier: 0 }).await; // 'i'
    // Now notify IMM to select candidate 0
    unsafe {
        let himc = adapter.sessions.get(&id).unwrap().himc;
        ImmNotifyIME(himc, NI_SELECTCANDIDATESTR, 0, 0); // candidate index 0
    }
    // Let's see if there is a commit
    if let Some(c) = adapter.get_commit(id).await {
        println!("Commit via Notify: {}", c);
    } else {
        println!("No commit via Notify");
    }
}
