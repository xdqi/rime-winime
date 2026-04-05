fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../grpc-contract-v2/proto/rime_service.proto");
    tonic_build::configure().compile_protos(
        &["../grpc-contract-v2/proto/rime_service.proto"],
        &["../grpc-contract-v2/proto"],
    )?;
    Ok(())
}
