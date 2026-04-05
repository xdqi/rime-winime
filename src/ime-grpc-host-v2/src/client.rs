pub mod proto {
    pub mod rime_service_v2 {
        tonic::include_proto!("rime.service.v2");
    }
}

use proto::rime_service_v2::KeyEvent;
use proto::rime_service_v2::{
    rime_service_client::RimeServiceClient, GetCommitRequest, GetContextRequest,
    OpenSessionRequest, ProcessKeyRequest,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let mut client = RimeServiceClient::connect("http://[::1]:50051").await?;

    tracing::info!("Connected to RimeService v2");

    // 1. Open session
    let response = client
        .open_session(OpenSessionRequest {
            schema_id: "luna_pinyin".to_string(),
        })
        .await?;
    let session_id = response.into_inner().session_id;
    tracing::info!("Opened session: {}", session_id);

    // 2. Process Key (e.g. 'a')
    let key_req = ProcessKeyRequest {
        session_id: session_id.clone(),
        key_event: Some(KeyEvent {
            keycode: 97, // 'a'
            modifier: 0,
        }),
    };
    let key_resp = client.process_key(key_req).await?;
    tracing::info!("ProcessKey accepted: {}", key_resp.into_inner().accepted);

    // 3. Get Context
    let ctx_resp = client
        .get_context(GetContextRequest {
            session_id: session_id.clone(),
        })
        .await?;
    tracing::info!("Context: {:#?}", ctx_resp.into_inner().context);

    // 4. Get Commit
    let commit_resp = client
        .get_commit(GetCommitRequest {
            session_id: session_id.clone(),
        })
        .await?;
    let inner = commit_resp.into_inner();
    tracing::info!(
        "Commit: text='{}', has_commit={}",
        inner.commit_text,
        inner.has_commit
    );

    Ok(())
}
