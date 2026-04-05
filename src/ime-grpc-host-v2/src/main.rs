// Modules are provided by the library crate
use ime_grpc_host_v2::proto;
use ime_grpc_host_v2::backend;
use ime_grpc_host_v2::server;

// Uncomment when developing win_imm
#[cfg(windows)]
use ime_grpc_host_v2::win_imm;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let addr = "127.0.0.1:50051".parse()?;

    use proto::rime_service_v2::rime_service_server::RimeServiceServer;
    use server::RimeServerImpl;
    #[cfg(not(windows))]
    let backend = Box::new(backend::native::NativeRimeBackend::new());

    #[cfg(windows)]
    let backend = Box::new(win_imm::ImmRimeAdapter::new());

    let server = RimeServerImpl::new(backend);

    tracing::info!("Starting RimeService v2 at {:?}", addr);

    tonic::transport::Server::builder()
        .add_service(RimeServiceServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}
