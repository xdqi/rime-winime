#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing...");
    let start = std::time::Instant::now();
    let mut client = crate::proto::rime_service_v2::rime_service_client::RimeServiceClient::connect("http://127.0.0.1:50051").await?;
    println!("Connected in {:?}", start.elapsed());
    
    let req = tonic::Request::new(crate::proto::rime_service_v2::OpenSessionRequest {
        schema_id: "luna_pinyin".into(),
    });
    
    let start2 = std::time::Instant::now();
    let resp = client.open_session(req).await?;
    println!("Opened session in {:?}", start2.elapsed());
    Ok(())
}
