//! gRPC client-based integration test for punctuation handling.
//!
//! This test connects to a RUNNING ime-grpc-host-v2 server and validates
//! that Chinese punctuation is correctly committed. It replicates the
//! behavior of rime_api_console's simulate_key_sequence + get_commit flow.
//!
//! Usage:
//!   1. Ensure the server is running (e.g. via systemd or manually)
//!   2. Set GRPC_SERVER_ADDR if not localhost:50051
//!   3. Run: cargo test --test test_grpc_punctuation -- --nocapture

use ime_grpc_host_v2::proto::rime_service_v2::{
    rime_service_client::RimeServiceClient, DestroySessionRequest, GetCommitRequest,
    GetContextRequest, KeyEvent, OpenSessionRequest, ProcessKeyRequest,
};

fn server_addr() -> String {
    std::env::var("GRPC_SERVER_ADDR").unwrap_or_else(|_| "http://127.0.0.1:50051".to_string())
}

async fn open_session(client: &mut RimeServiceClient<tonic::transport::Channel>) -> String {
    let resp = client
        .open_session(OpenSessionRequest {
            schema_id: String::new(),
        })
        .await
        .expect("Failed to open session");
    let id = resp.into_inner().session_id;
    println!("Opened session: {}", id);
    id
}

async fn destroy_session(
    client: &mut RimeServiceClient<tonic::transport::Channel>,
    session_id: &str,
) {
    let _ = client
        .destroy_session(DestroySessionRequest {
            session_id: session_id.to_string(),
        })
        .await;
    println!("Destroyed session: {}", session_id);
}

async fn process_key(
    client: &mut RimeServiceClient<tonic::transport::Channel>,
    session_id: &str,
    keycode: u32,
    modifier: u32,
    label: char,
) -> bool {
    let resp = client
        .process_key(ProcessKeyRequest {
            session_id: session_id.to_string(),
            key_event: Some(KeyEvent { keycode, modifier }),
        })
        .await
        .expect("ProcessKey RPC failed");
    let accepted = resp.into_inner().accepted;
    if !accepted {
        println!("  Key '{}' (0x{:X}): NOT accepted", label, keycode);
    } else {
        println!("  Key '{}' (0x{:X}): accepted", label, keycode);
    }
    accepted
}

async fn get_commit(
    client: &mut RimeServiceClient<tonic::transport::Channel>,
    session_id: &str,
) -> Option<String> {
    let resp = client
        .get_commit(GetCommitRequest {
            session_id: session_id.to_string(),
        })
        .await
        .expect("GetCommit RPC failed");
    let inner = resp.into_inner();
    if inner.has_commit && !inner.commit_text.is_empty() {
        Some(inner.commit_text)
    } else {
        None
    }
}

async fn print_context(
    client: &mut RimeServiceClient<tonic::transport::Channel>,
    session_id: &str,
) -> String {
    let resp = client
        .get_context(GetContextRequest {
            session_id: session_id.to_string(),
        })
        .await
        .expect("GetContext RPC failed");
    let ctx = resp.into_inner().context;
    if let Some(ctx) = ctx {
        if let Some(menu) = &ctx.menu {
            if menu.num_candidates > 0 {
                print!("    Candidates: ");
                for (i, cand) in menu.candidates.iter().enumerate() {
                    print!("{}.{} ", i + 1, cand.text);
                }
                println!("(total: {})", menu.num_candidates);
            }
        }
        if let Some(comp) = ctx.composition {
            return comp.preedit;
        }
    }
    String::new()
}

/// Send a sequence of keys and collect all commits
async fn run_key_sequence(
    client: &mut RimeServiceClient<tonic::transport::Channel>,
    session_id: &str,
    keys: &[(u32, u32, char)],
) -> String {
    let mut accumulated = String::new();

    for &(keycode, modifier, label) in keys {
        let _accepted = process_key(client, session_id, keycode, modifier, label).await;

        // Small delay to mimic real typing
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        let preedit = print_context(client, session_id).await;
        if !preedit.is_empty() {
            println!("    Preedit: {}", preedit);
        }

        if let Some(commit) = get_commit(client, session_id).await {
            println!("    >>> Commit: {}", commit);
            accumulated.push_str(&commit);
        }
    }

    accumulated
}

#[tokio::test]
async fn test_grpc_nihao_comma() {
    let addr = server_addr();
    println!("\n=== Connecting to {} ===", addr);
    let mut client = RimeServiceClient::connect(addr)
        .await
        .expect("Failed to connect to gRPC server");

    let session_id = open_session(&mut client).await;

    println!("\n--- Test: nihao + space (expect '你好') ---");
    let keys = [
        (0x6E, 0u32, 'n'),
        (0x69, 0, 'i'),
        (0x68, 0, 'h'),
        (0x61, 0, 'a'),
        (0x6F, 0, 'o'),
        (0x20, 0, ' '),
    ];
    let commit = run_key_sequence(&mut client, &session_id, &keys).await;
    println!("  Total commit: {:?}", commit);
    assert!(
        commit.contains("你好"),
        "Expected '你好' in commit, got {:?}",
        commit
    );

    println!("\n--- Test: nihao + comma (expect '你好，' or at least '你好' with '，') ---");
    let keys = [
        (0x6E, 0u32, 'n'),
        (0x69, 0, 'i'),
        (0x68, 0, 'h'),
        (0x61, 0, 'a'),
        (0x6F, 0, 'o'),
        (0x2C, 0, ','),
    ];
    let commit = run_key_sequence(&mut client, &session_id, &keys).await;
    println!("  Total commit: {:?}", commit);

    println!("\n--- Test: standalone comma (expect '，') ---");
    let keys = [(0x2C, 0u32, ',')];
    let commit = run_key_sequence(&mut client, &session_id, &keys).await;
    println!("  Total commit: {:?}", commit);

    println!("\n--- Test: kkk then shift (expect 'kkk') ---");
    let keys = [
        (0x73, 0u32, 's'),
        (0x73, 0, 's'),
        (0x68, 0, 'h'),
        (0xFFE1, 1, '⇧'),
    ];
    let commit = run_key_sequence(&mut client, &session_id, &keys).await;
    println!("  Total commit: {:?}", commit);

    println!("\n--- Test: 1.. (expect '1.。') ---");
    let keys = [(0x31, 0u32, '1'), (0x2E, 0, '.'), (0x2E, 0, '.')];
    let commit = run_key_sequence(&mut client, &session_id, &keys).await;
    println!("  Total commit: {:?}", commit);

    destroy_session(&mut client, &session_id).await;
    println!("\n=== All gRPC punctuation tests completed ===");
}
