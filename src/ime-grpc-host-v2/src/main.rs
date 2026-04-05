pub mod proto {
    pub mod rime_service_v2 {
        tonic::include_proto!("rime.service.v2");
    }
}

pub mod backend;
pub mod server;

// Uncomment when developing win_imm
#[cfg(windows)]
pub mod win_imm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let addr = "127.0.0.1:50051".parse()?;

    use proto::rime_service_v2::rime_service_server::RimeServiceServer;
    use server::RimeServerImpl;
    #[cfg(not(windows))]
    let backend = Box::new(backend::native::NativeRimeBackend::new());

    #[cfg(windows)]
    let backend = Box::new(win_imm::channel_adapter::ChannelRimeBackend::new(None));

    let server = RimeServerImpl::new(backend);

    tracing::info!("Starting RimeService v2 at {:?}", addr);

    tonic::transport::Server::builder()
        .add_service(RimeServiceServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}
