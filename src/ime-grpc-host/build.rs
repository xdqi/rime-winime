fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "../grpc-contract/proto/ime_proxy.proto";
    let include = "../grpc-contract/proto";

    println!("cargo:rerun-if-changed={proto}");

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&[proto], &[include])?;

    Ok(())
}
