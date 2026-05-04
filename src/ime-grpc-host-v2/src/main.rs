// Modules are provided by the library crate
use clap::Parser;
use ime_grpc_host_v2::proto;
use ime_grpc_host_v2::server;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The IP and port to listen on for gRPC connections
    #[arg(long, env = "GRPC_BIND_ADDR", default_value = "127.0.0.1:50051")]
    bind: String,

    /// Path to the IME DLL (e.g. C:\Windows\system32\SogouPY.ime or C:\Windows\system32\QQPinyin.ime)
    #[arg(
        long,
        env = "GRPC_IME_PATH",
        default_value = "C:\\Windows\\system32\\SogouPY.ime"
    )]
    ime_path: String,

    /// Whether to show the hidden message window for debugging
    #[arg(long, env = "GRPC_SHOW_WINDOW", default_value_t = false)]
    show_window: bool,

    /// Auto-destroy idle sessions after this many seconds
    #[arg(long, env = "GRPC_SESSION_TIMEOUT_SEC", default_value_t = 600)]
    session_timeout_sec: u64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let addr = args.bind.parse()?;

    use proto::rime_service_v2::rime_service_server::RimeServiceServer;
    use server::RimeServerImpl;
    #[cfg(not(windows))]
    let backend = Box::new(ime_grpc_host_v2::backend::native::NativeRimeBackend::new());

    #[cfg(windows)]
    let backend = Box::new(ime_grpc_host_v2::win_imm::ImmRimeAdapter::new(
        &args.ime_path,
        args.show_window,
    ));

    let server = RimeServerImpl::new(backend);

    tracing::info!("Starting RimeService v2 at {:?}", addr);

    tonic::transport::Server::builder()
        .add_service(RimeServiceServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}
