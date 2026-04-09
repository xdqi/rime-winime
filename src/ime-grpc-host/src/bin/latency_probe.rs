use std::env;
use std::time::Instant;

use tonic::Request;

pub mod proto {
    tonic::include_proto!("ime.gateway.v1");
}

use proto::ime_gateway_client::ImeGatewayClient;
use proto::{KeyEvent, OpenSessionRequest, SendKeyEventRequest};

fn env_usize(name: &str, fallback: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(fallback)
}

fn env_f64(name: &str, fallback: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(fallback)
}

fn env_bool(name: &str, fallback: bool) -> bool {
    let raw = match env::var(name) {
        Ok(v) => v,
        Err(_) => return fallback,
    };

    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => fallback,
    }
}

fn percentile_us(sorted_us: &[u128], quantile: f64) -> u128 {
    if sorted_us.is_empty() {
        return 0;
    }

    let clamped = quantile.clamp(0.0, 1.0);
    let n = sorted_us.len();
    let rank = (clamped * n as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted_us[idx]
}

fn us_to_ms(value_us: u128) -> f64 {
    value_us as f64 / 1000.0
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint =
        env::var("IME_GRPC_ENDPOINT").unwrap_or_else(|_| "http://127.0.0.1:50051".to_string());
    let rounds = env_usize("IME_BENCH_ROUNDS", 1000);
    let input = env::var("IME_BENCH_INPUT").unwrap_or_else(|_| "nihao".to_string());
    let frontend_id =
        env::var("IME_BENCH_FRONTEND_ID").unwrap_or_else(|_| "ime-grpc-latency".to_string());
    let schema_id = env::var("IME_BENCH_SCHEMA_ID").unwrap_or_else(|_| "grpc_proxy".to_string());
    let want_prewarmed = env_bool("IME_BENCH_WANT_PREWARMED", true);
    let target_p95_ms = env_f64("IME_BENCH_TARGET_P95_MS", 0.0);

    if rounds == 0 {
        return Err("IME_BENCH_ROUNDS must be greater than 0".into());
    }

    let input_keycodes: Vec<u32> = input
        .chars()
        .filter(|ch| ch.is_ascii())
        .map(|ch| ch as u32)
        .collect();

    if input_keycodes.is_empty() {
        return Err("IME_BENCH_INPUT must contain at least one ASCII character".into());
    }

    let mut client = ImeGatewayClient::connect(endpoint.clone()).await?;

    let open = client
        .open_session(Request::new(OpenSessionRequest {
            frontend_id,
            schema_id,
            want_prewarmed_worker: want_prewarmed,
        }))
        .await?
        .into_inner();

    println!(
        "open_session: endpoint={} session_id={} worker_id={} backend_state_version={} want_prewarmed={}",
        endpoint, open.session_id, open.worker_id, open.backend_state_version, want_prewarmed
    );

    let mut latencies_us: Vec<u128> = Vec::with_capacity(rounds);

    for i in 0..rounds {
        let seq = (i as u64) + 1;
        let vk = input_keycodes[i % input_keycodes.len()];

        let start = Instant::now();
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
        let elapsed = start.elapsed().as_micros();

        if !reply.error_code.is_empty() {
            return Err(format!(
                "send_key_event failed at seq={} error_code={} error_message={}",
                seq, reply.error_code, reply.error_message
            )
            .into());
        }

        latencies_us.push(elapsed);
    }

    latencies_us.sort_unstable();

    let sum_us: u128 = latencies_us.iter().copied().sum();
    let avg_us = sum_us as f64 / latencies_us.len() as f64;

    let p50_us = percentile_us(&latencies_us, 0.50);
    let p95_us = percentile_us(&latencies_us, 0.95);
    let p99_us = percentile_us(&latencies_us, 0.99);

    println!(
        "latency_stats rounds={} want_prewarmed={} p50_ms={:.3} p95_ms={:.3} p99_ms={:.3} avg_ms={:.3}",
        rounds,
        want_prewarmed,
        us_to_ms(p50_us),
        us_to_ms(p95_us),
        us_to_ms(p99_us),
        avg_us / 1000.0
    );

    if target_p95_ms > 0.0 && us_to_ms(p95_us) > target_p95_ms {
        eprintln!(
            "latency_target_failed target_p95_ms={:.3} actual_p95_ms={:.3}",
            target_p95_ms,
            us_to_ms(p95_us)
        );
        std::process::exit(2);
    }

    Ok(())
}
