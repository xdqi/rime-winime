//! TSF diagnostics: stderr + tracing so `cargo test --nocapture` shows progress without RUST_LOG.

pub fn tsf_step(msg: impl std::fmt::Display) {
    let s = msg.to_string();
    eprintln!("{s}");
    tracing::info!("{s}");
}

pub fn tsf_warn(msg: impl std::fmt::Display) {
    let s = msg.to_string();
    eprintln!("{s}");
    tracing::warn!("{s}");
}
