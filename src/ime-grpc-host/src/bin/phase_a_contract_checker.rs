use std::env;

use tonic::{Code, Request, Status};

pub mod proto {
    tonic::include_proto!("ime.gateway.v1");
}

use proto::ime_gateway_client::ImeGatewayClient;
use proto::{
    CommitSelectionRequest, GetStatusRequest, KeyEvent, OpenSessionRequest, PingRequest,
    QueryCandidatesRequest, ResetSessionRequest, SendKeyEventRequest,
};

fn assert_true(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_string())
    }
}

fn expect_status_code<T>(
    result: Result<tonic::Response<T>, Status>,
    expected: Code,
    context: &str,
) -> Result<(), String> {
    match result {
        Ok(_) => Err(format!(
            "{}: expected status {:?}, got success",
            context, expected
        )),
        Err(status) if status.code() == expected => Ok(()),
        Err(status) => Err(format!(
            "{}: expected status {:?}, got {:?}: {}",
            context,
            expected,
            status.code(),
            status.message()
        )),
    }
}

fn make_key_event(seq: u64, key: char) -> KeyEvent {
    let vk = key as u32;
    KeyEvent {
        seq,
        key_down: true,
        virtual_key: vk,
        scan_code: 0x1e,
        shift: false,
        ctrl: false,
        alt: false,
        repeated: false,
        extended: false,
        timestamp_ms: 1700000000000,
        source_keycode: vk,
        source_modifier: 0x7,
    }
}

async fn run() -> Result<(), String> {
    let endpoint =
        env::var("IME_GRPC_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:50051".to_string());

    println!("[phase] connect endpoint={}", endpoint);
    let mut client = ImeGatewayClient::connect(endpoint)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;

    println!("[phase] ping/get_status sanity");
    let ping = client
        .ping(Request::new(PingRequest {
            payload: "phase-a-contract-check".to_string(),
        }))
        .await
        .map_err(|e| format!("ping failed: {e}"))?
        .into_inner();
    assert_true(
        ping.payload == "phase-a-contract-check",
        "ping payload mismatch",
    )?;
    assert_true(ping.server_unix_ms > 0, "ping timestamp must be positive")?;

    let status_before = client
        .get_status(Request::new(GetStatusRequest {
            session_id: String::new(),
        }))
        .await
        .map_err(|e| format!("get_status before open failed: {e}"))?
        .into_inner();
    assert_true(status_before.ok, "get_status before open must return ok=true")?;

    println!("[phase] negative case: missing key_event -> invalid_argument");
    expect_status_code(
        client
            .send_key_event(Request::new(SendKeyEventRequest {
                session_id: "irrelevant".to_string(),
                key_event: None,
            }))
            .await,
        Code::InvalidArgument,
        "send_key_event without key_event",
    )?;

    println!("[phase] open session and happy path RPC chain");
    let open1 = client
        .open_session(Request::new(OpenSessionRequest {
            frontend_id: "phase-a-checker".to_string(),
            schema_id: "grpc_proxy".to_string(),
            want_prewarmed_worker: true,
        }))
        .await
        .map_err(|e| format!("open_session #1 failed: {e}"))?
        .into_inner();

    assert_true(!open1.session_id.is_empty(), "open_session #1 session_id is empty")?;
    assert_true(!open1.worker_id.is_empty(), "open_session #1 worker_id is empty")?;

    let send1 = client
        .send_key_event(Request::new(SendKeyEventRequest {
            session_id: open1.session_id.clone(),
            key_event: Some(make_key_event(1, 'n')),
        }))
        .await
        .map_err(|e| format!("send_key_event seq=1 failed: {e}"))?
        .into_inner();

    assert_true(
        send1.session_id == open1.session_id,
        "send_key_event session_id mismatch",
    )?;
    assert_true(send1.acknowledged_seq == 1, "send_key_event ack seq must be 1")?;
    if !send1.error_code.is_empty() || !send1.error_message.is_empty() {
        return Err(format!(
            "send_key_event seq=1 unexpected business error: error_code={} error_message={} composition={} backend_state_version={}",
            send1.error_code,
            send1.error_message,
            send1.composition,
            send1.backend_state_version
        ));
    }

    println!("[phase] negative case: seq out of order");
    let send_dup = client
        .send_key_event(Request::new(SendKeyEventRequest {
            session_id: open1.session_id.clone(),
            key_event: Some(make_key_event(1, 'n')),
        }))
        .await
        .map_err(|e| format!("send_key_event duplicate seq failed: {e}"))?
        .into_inner();

    assert_true(
        send_dup.error_code == "SEQ_OUT_OF_ORDER",
        "duplicate seq must return SEQ_OUT_OF_ORDER",
    )?;
    assert_true(
        !send_dup.error_message.is_empty(),
        "duplicate seq must return non-empty error_message",
    )?;

    let query = client
        .query_candidates(Request::new(QueryCandidatesRequest {
            session_id: open1.session_id.clone(),
            seq: 2,
            input_snapshot: "n".to_string(),
            max_candidates: 5,
        }))
        .await
        .map_err(|e| format!("query_candidates failed: {e}"))?
        .into_inner();

    assert_true(
        query.session_id == open1.session_id,
        "query_candidates session_id mismatch",
    )?;
    assert_true(
        query.error_code.is_empty(),
        "query_candidates should have empty error_code",
    )?;

    let commit = client
        .commit_selection(Request::new(CommitSelectionRequest {
            session_id: open1.session_id.clone(),
            seq: 3,
            candidate_index: 0,
            committed_text: String::new(),
        }))
        .await
        .map_err(|e| format!("commit_selection failed: {e}"))?
        .into_inner();

    assert_true(
        commit.error_code.is_empty(),
        "commit_selection should have empty error_code",
    )?;

    let reset = client
        .reset_session(Request::new(ResetSessionRequest {
            session_id: open1.session_id.clone(),
            reason: "phase-a-check".to_string(),
        }))
        .await
        .map_err(|e| format!("reset_session failed: {e}"))?
        .into_inner();

    assert_true(reset.ok, "reset_session should return ok=true")?;
    assert_true(
        reset.error_code.is_empty(),
        "reset_session should have empty error_code",
    )?;

    println!("[phase] negative case: unknown session -> not_found");
    let missing_session = "phase-a-missing-session".to_string();

    expect_status_code(
        client
            .send_key_event(Request::new(SendKeyEventRequest {
                session_id: missing_session.clone(),
                key_event: Some(make_key_event(1, 'x')),
            }))
            .await,
        Code::NotFound,
        "send_key_event with missing session",
    )?;

    expect_status_code(
        client
            .query_candidates(Request::new(QueryCandidatesRequest {
                session_id: missing_session.clone(),
                seq: 1,
                input_snapshot: "x".to_string(),
                max_candidates: 5,
            }))
            .await,
        Code::NotFound,
        "query_candidates with missing session",
    )?;

    expect_status_code(
        client
            .commit_selection(Request::new(CommitSelectionRequest {
                session_id: missing_session.clone(),
                seq: 1,
                candidate_index: 0,
                committed_text: String::new(),
            }))
            .await,
        Code::NotFound,
        "commit_selection with missing session",
    )?;

    expect_status_code(
        client
            .reset_session(Request::new(ResetSessionRequest {
                session_id: missing_session,
                reason: "phase-a-check".to_string(),
            }))
            .await,
        Code::NotFound,
        "reset_session with missing session",
    )?;

    println!("[phase] open second session uniqueness and status accounting");
    let open2 = client
        .open_session(Request::new(OpenSessionRequest {
            frontend_id: "phase-a-checker-2".to_string(),
            schema_id: "grpc_proxy".to_string(),
            want_prewarmed_worker: true,
        }))
        .await
        .map_err(|e| format!("open_session #2 failed: {e}"))?
        .into_inner();

    assert_true(open2.session_id != open1.session_id, "session_id must be unique")?;

    let status_after = client
        .get_status(Request::new(GetStatusRequest {
            session_id: String::new(),
        }))
        .await
        .map_err(|e| format!("get_status after checks failed: {e}"))?
        .into_inner();

    assert_true(status_after.ok, "get_status after checks must return ok=true")?;
    assert_true(
        status_after.active_sessions >= 2,
        "active_sessions should be >= 2 after two open sessions",
    )?;

    println!("[result] PASS");
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(reason) = run().await {
        eprintln!("[result] FAIL reason={}", reason);
        std::process::exit(1);
    }
}
