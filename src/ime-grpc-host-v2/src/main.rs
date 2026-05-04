// Modules are provided by the library crate
use clap::Parser;
use clap::ValueEnum;
use ime_grpc_host_v2::proto;
use ime_grpc_host_v2::server;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum ImeType {
    /// IMM32-based IME (`.ime` DLL path)
    #[default]
    Imm,
    /// Text Services Framework TIP (CLSID / description / DLL name)
    Tsf,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The IP and port to listen on for gRPC connections
    #[arg(long, env = "GRPC_BIND_ADDR", default_value = "127.0.0.1:50051")]
    bind: String,

    /// Backend: `imm` (IMM32) or `tsf` (TSF TIP)
    #[arg(long, env = "GRPC_IME_TYPE", value_enum, default_value_t = ImeType::Imm)]
    ime_type: ImeType,

    /// Path to the IME DLL (e.g. C:\Windows\system32\SogouPY.ime or C:\Windows\system32\QQPinyin.ime)
    #[arg(
        long,
        env = "GRPC_IME_PATH",
        default_value = "C:\\Windows\\system32\\SogouPY.ime"
    )]
    ime_path: String,

    /// TIP class id (registry string form), e.g. `{GUID}`
    #[arg(long, env = "GRPC_TIP_CLSID")]
    tip_clsid: Option<String>,

    /// TIP description substring (matched against registry Display Name)
    #[arg(long, env = "GRPC_TIP_NAME")]
    tip_name: Option<String>,

    /// TIP IME file name only, e.g. `SogouPY.ime`
    #[arg(long, env = "GRPC_TIP_DLL")]
    tip_dll: Option<String>,

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
    let backend: Box<dyn ime_grpc_host_v2::backend::RimeBackend> = match args.ime_type {
        ImeType::Imm => Box::new(ime_grpc_host_v2::win_imm::ImmRimeAdapter::new(
            &args.ime_path,
            args.show_window,
        )),
        ImeType::Tsf => {
            let adapter = unsafe {
                ime_grpc_host_v2::win_tsf::TsfRimeAdapter::from_options(
                    args.tip_clsid.as_deref(),
                    args.tip_name.as_deref(),
                    args.tip_dll.as_deref(),
                )
            }
            .map_err(|e| format!("TSF backend init failed: {}", e))?;
            Box::new(adapter)
        }
    };

    let server = RimeServerImpl::new(backend);

    tracing::info!("Starting RimeService v2 at {:?}", addr);

    tonic::transport::Server::builder()
        .add_service(RimeServiceServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}
