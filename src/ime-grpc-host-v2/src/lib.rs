pub mod backend;
pub mod proto {
    pub mod rime_service_v2 {
        tonic::include_proto!("rime.service.v2");
    }
}

#[cfg(all(windows, feature = "imm-backend"))]
pub mod win_imm;

#[cfg(windows)]
pub mod win_tsf;

#[cfg(windows)]
pub mod win_keymap;

pub mod server;
