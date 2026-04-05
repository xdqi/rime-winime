pub mod backend;
pub mod proto {
    pub mod rime_service_v2 {
        tonic::include_proto!("rime.service.v2");
    }
}


#[cfg(windows)]
pub mod win_imm;

pub mod server;
