use std::env;

use tonic::Request;

pub mod proto {
    tonic::include_proto!("ime.gateway.v1");
}

use proto::ime_gateway_client::ImeGatewayClient;
use proto::{
    CommitSelectionRequest, KeyEvent, OpenSessionRequest, QueryCandidatesRequest,
    SendKeyEventRequest,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint =
        env::var("IME_GRPC_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:50051".to_string());
    let input = env::var("IME_SMOKE_INPUT").unwrap_or_else(|_| "rime".to_string());

    let mut client = ImeGatewayClient::connect(endpoint.clone()).await?;

    let open = client
        .open_session(Request::new(OpenSessionRequest {
            frontend_id: "ime-grpc-smoke".to_string(),
            schema_id: "grpc_proxy".to_string(),
            want_prewarmed_worker: true,
        }))
        .await?
        .into_inner();

    println!(
        "open_session: session_id={} worker_id={} backend_state_version={}",
        open.session_id, open.worker_id, open.backend_state_version
    );

    let mut seq: u64 = 0;
    for ch in input.chars() {
        if !ch.is_ascii() {
            continue;
        }

        seq += 1;
        let vk = ch as u32;

        let reply = client
            .send_key_event(Request::new(SendKeyEventRequest {
                session_id: open.session_id.clone(),
                key_event: Some(KeyEvent {
                    seq,
                    key_down: true,
                    virtual_key: vk,
                    scan_code: 0,
                    shift: false,
                    ctrl: false,
                    alt: false,
                    repeated: false,
                    extended: false,
                    timestamp_ms: 0,
                    source_keycode: vk,
                    source_modifier: 0,
                }),
            }))
            .await?
            .into_inner();

        println!(
            "send_key_event: seq={} composition='{}' error_code='{}' error_message='{}'",
            reply.acknowledged_seq,
            reply.composition,
            reply.error_code,
            reply.error_message
        );
    }

    let query = client
        .query_candidates(Request::new(QueryCandidatesRequest {
            session_id: open.session_id.clone(),
            seq,
            input_snapshot: String::new(),
            max_candidates: 9,
        }))
        .await?
        .into_inner();

    println!(
        "query_candidates: composition='{}' reading='{}' count={} error_code='{}' error_message='{}'",
        query.composition,
        query.reading,
        query.candidates.len(),
        query.error_code,
        query.error_message
    );

    for item in &query.candidates {
        println!(
            "  - [{}] {} ({}, q={:.2})",
            item.index, item.text, item.comment, item.quality
        );
    }

    if !query.candidates.is_empty() {
        let commit = client
            .commit_selection(Request::new(CommitSelectionRequest {
                session_id: open.session_id,
                seq: seq + 1,
                candidate_index: 0,
                committed_text: String::new(),
            }))
            .await?
            .into_inner();

        println!(
            "commit_selection: committed='{}' error_code='{}' error_message='{}'",
            commit.committed_text,
            commit.error_code,
            commit.error_message
        );
    }

    Ok(())
}
