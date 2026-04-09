#![deny(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::env;
use std::io::{self, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod backend;

use backend::{
    win_imm::WinImmBackend, BackendCandidate, BackendCommitResult, BackendEventResult,
    BackendKeyEvent, BackendQueryResult, BackendSnapshot, ImeBackend,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tonic::{transport::Server, Request, Response, Status};
use tracing::{info, warn};
use uuid::Uuid;

pub mod proto {
    tonic::include_proto!("ime.gateway.v1");
}

use proto::ime_gateway_server::{ImeGateway, ImeGatewayServer};
use proto::{
    CandidateItem, CommitSelectionRequest, CommitSelectionResponse, GetStatusRequest,
    GetStatusResponse, OpenSessionRequest, OpenSessionResponse, PingRequest, PingResponse,
    QueryCandidatesRequest, QueryCandidatesResponse, ResetSessionRequest, ResetSessionResponse,
    SendKeyEventRequest, SendKeyEventResponse,
};

#[derive(Clone, Debug)]
struct PoolConfig {
    min_idle: usize,
    max_idle: usize,
    prewarm: bool,
    spawn_timeout_ms: u64,
}

impl PoolConfig {
    fn from_env() -> Self {
        Self {
            min_idle: env_usize("IME_POOL_MIN_IDLE", 1),
            max_idle: env_usize("IME_POOL_MAX_IDLE", 4),
            prewarm: env_bool("IME_POOL_PREWARM", true),
            spawn_timeout_ms: env_u64("IME_POOL_SPAWN_TIMEOUT_MS", 1500),
        }
    }
}

#[derive(Clone, Debug)]
struct WorkerHandle {
    worker_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkerIpcCandidate {
    index: u32,
    text: String,
    comment: String,
    quality: f64,
}

impl WorkerIpcCandidate {
    fn into_proto(self) -> CandidateItem {
        CandidateItem {
            index: self.index,
            text: self.text,
            comment: self.comment,
            quality: self.quality,
        }
    }
}

impl From<CandidateItem> for WorkerIpcCandidate {
    fn from(value: CandidateItem) -> Self {
        Self {
            index: value.index,
            text: value.text,
            comment: value.comment,
            quality: value.quality,
        }
    }
}

impl From<WorkerIpcCandidate> for BackendCandidate {
    fn from(value: WorkerIpcCandidate) -> Self {
        Self {
            index: value.index,
            text: value.text,
            comment: value.comment,
            quality: value.quality,
        }
    }
}

impl From<BackendCandidate> for WorkerIpcCandidate {
    fn from(value: BackendCandidate) -> Self {
        Self {
            index: value.index,
            text: value.text,
            comment: value.comment,
            quality: value.quality,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkerIpcRequest {
    op: String,
    #[serde(default)]
    trace_timeline: bool,
    #[serde(default)]
    key_down: bool,
    #[serde(default)]
    virtual_key: u32,
    #[serde(default)]
    scan_code: u32,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    repeated: bool,
    #[serde(default)]
    extended: bool,
    #[serde(default)]
    timestamp_ms: i64,
    #[serde(default)]
    source_keycode: u32,
    #[serde(default)]
    source_modifier: u32,
    #[serde(default)]
    max_candidates: usize,
    #[serde(default)]
    input_snapshot: String,
    #[serde(default)]
    committed_text: String,
    #[serde(default)]
    candidate_index: usize,
}

fn worker_trace_timeline_enabled() -> bool {
    env_bool("IME_WINIMM_TRACE_TIMELINE", false)
}

impl WorkerIpcRequest {
    fn snapshot() -> Self {
        Self {
            op: "snapshot".to_string(),
            trace_timeline: worker_trace_timeline_enabled(),
            key_down: false,
            virtual_key: 0,
            scan_code: 0,
            shift: false,
            ctrl: false,
            alt: false,
            repeated: false,
            extended: false,
            timestamp_ms: 0,
            source_keycode: 0,
            source_modifier: 0,
            max_candidates: 0,
            input_snapshot: String::new(),
            committed_text: String::new(),
            candidate_index: 0,
        }
    }

    fn reset_session() -> Self {
        Self {
            op: "reset_session".to_string(),
            ..Self::snapshot()
        }
    }

    fn send_key_event(key_event: &BackendKeyEvent, max_candidates: usize) -> Self {
        Self {
            op: "send_key_event".to_string(),
            key_down: key_event.key_down,
            virtual_key: key_event.virtual_key,
            scan_code: key_event.scan_code,
            shift: key_event.shift,
            ctrl: key_event.ctrl,
            alt: key_event.alt,
            repeated: key_event.repeated,
            extended: key_event.extended,
            timestamp_ms: key_event.timestamp_ms,
            source_keycode: key_event.source_keycode,
            source_modifier: key_event.source_modifier,
            max_candidates,
            ..Self::snapshot()
        }
    }

    fn query_candidates(input_snapshot: String, max_candidates: usize) -> Self {
        Self {
            op: "query_candidates".to_string(),
            input_snapshot,
            max_candidates,
            ..Self::snapshot()
        }
    }

    fn commit_selection(committed_text: String, candidate_index: usize) -> Self {
        Self {
            op: "commit_selection".to_string(),
            committed_text,
            candidate_index,
            ..Self::snapshot()
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct WorkerIpcResponse {
    ok: bool,
    #[serde(default)]
    backend_state_version: u64,
    #[serde(default)]
    composition: String,
    #[serde(default)]
    reading: String,
    #[serde(default)]
    candidates: Vec<WorkerIpcCandidate>,
    #[serde(default)]
    selected_index: u32,
    #[serde(default)]
    page_size: u32,
    #[serde(default)]
    committed_text: String,
    #[serde(default)]
    error_code: String,
    #[serde(default)]
    error_message: String,
    #[serde(default)]
    debug_timeline: Vec<String>,
}

const IPC_MAX_FRAME_BYTES: usize = 256 * 1024;

fn write_framed_payload<W: Write>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    if payload.len() > IPC_MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "payload too large: {} > {}",
                payload.len(),
                IPC_MAX_FRAME_BYTES
            ),
        ));
    }

    let len = payload.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

fn read_framed_payload<R: Read>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0_u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }

    let len = u32::from_le_bytes(len_buf) as usize;
    if len > IPC_MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid frame length: {len}"),
        ));
    }

    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload)?;
    Ok(Some(payload))
}

fn worker_error_response(snapshot: WorkerSnapshot, code: &str, message: &str) -> WorkerIpcResponse {
    WorkerIpcResponse {
        ok: false,
        backend_state_version: snapshot.backend_state_version,
        composition: snapshot.composition,
        reading: String::new(),
        candidates: Vec::new(),
        selected_index: 0,
        page_size: 0,
        committed_text: String::new(),
        error_code: code.to_string(),
        error_message: message.to_string(),
        debug_timeline: Vec::new(),
    }
}

fn extract_between(text: &str, begin: &str, end_char: char, from: usize) -> Option<String> {
    let p = text[from..].find(begin)? + from + begin.len();
    let q = text[p..].find(end_char)? + p;
    Some(text[p..q].to_string())
}

fn parse_candidate_reply(
    line: &str,
    max_candidates: usize,
) -> Result<(String, String, Vec<WorkerIpcCandidate>), String> {
    if !line.starts_with("CAND_RET ") {
        return Err(format!("unexpected reply: {line}"));
    }

    if let Some(p) = line.find("err=") {
        let err = line[p + 4..]
            .split_whitespace()
            .next()
            .unwrap_or("unknown");
        return Err(format!("legacy host returned error: {err}"));
    }

    let composition = extract_between(line, "comp=[", ']', 0).unwrap_or_default();
    let reading = extract_between(line, "read=[", ']', 0).unwrap_or_default();
    let raw_items = extract_between(line, "items=[", ']', 0).unwrap_or_default();

    let mut candidates = Vec::new();
    for (idx, token) in raw_items
        .split('|')
        .filter(|s| !s.is_empty() && *s != "...")
        .take(max_candidates)
        .enumerate()
    {
        candidates.push(WorkerIpcCandidate {
            index: idx as u32,
            text: token.to_string(),
            comment: "legacy".to_string(),
            quality: (100.0 - idx as f64).max(1.0),
        });
    }

    Ok((composition, reading, candidates))
}

#[derive(Debug)]
struct LegacyTcpRuntime {
    host: String,
    port: u16,
    codepage: i32,
    command: String,
    timeout_ms: u64,
    stream: Option<TcpStream>,
    activated: bool,
    input: String,
    composition: String,
    reading: String,
    candidates: Vec<WorkerIpcCandidate>,
    last_commit: String,
    backend_state_version: u64,
}

impl LegacyTcpRuntime {
    fn from_env() -> Self {
        Self {
            host: env::var("IME_LEGACY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: env_u64("IME_LEGACY_PORT", 22345) as u16,
            codepage: env_u64("IME_LEGACY_CODEPAGE", 936) as i32,
            command: env::var("IME_LEGACY_COMMAND").unwrap_or_else(|_| "TEXTU".to_string()),
            timeout_ms: env_u64("IME_LEGACY_TIMEOUT_MS", 1200),
            stream: None,
            activated: false,
            input: String::new(),
            composition: String::new(),
            reading: String::new(),
            candidates: Vec::new(),
            last_commit: String::new(),
            backend_state_version: 1,
        }
    }

    fn clear_local_state(&mut self) {
        self.input.clear();
        self.composition.clear();
        self.reading.clear();
        self.candidates.clear();
        self.last_commit.clear();
    }

    fn reset_connection(&mut self) {
        self.stream = None;
        self.activated = false;
    }

    fn connect_if_needed(&mut self) -> Result<(), String> {
        if self.stream.is_some() {
            return Ok(());
        }

        let addr = format!("{}:{}", self.host, self.port);
        let stream = TcpStream::connect(&addr).map_err(|e| format!("connect {addr} failed: {e}"))?;
        let timeout = Duration::from_millis(self.timeout_ms.max(1));
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| format!("set_read_timeout failed: {e}"))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| format!("set_write_timeout failed: {e}"))?;
        stream
            .set_nodelay(true)
            .map_err(|e| format!("set_nodelay failed: {e}"))?;

        self.stream = Some(stream);
        self.activated = false;
        Ok(())
    }

    fn read_line(stream: &mut TcpStream) -> Result<Option<String>, String> {
        let mut out = Vec::new();
        let mut byte = [0_u8; 1];

        loop {
            match stream.read(&mut byte) {
                Ok(0) => {
                    if out.is_empty() {
                        return Ok(None);
                    }
                    break;
                }
                Ok(_) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    if byte[0] != b'\r' {
                        out.push(byte[0]);
                    }
                    if out.len() > IPC_MAX_FRAME_BYTES {
                        return Err("line exceeds max size".to_string());
                    }
                }
                Err(err) => {
                    return Err(format!("socket read failed: {err}"));
                }
            }
        }

        Ok(Some(String::from_utf8_lossy(&out).to_string()))
    }

    fn send_command_raw(&mut self, command: &str, expected_prefix: &str) -> Result<String, String> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| "legacy stream not connected".to_string())?;

        stream
            .write_all(command.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .and_then(|_| stream.flush())
            .map_err(|e| format!("socket write failed: {e}"))?;

        let mut last = String::new();
        for _ in 0..16 {
            let line = Self::read_line(stream)?
                .ok_or_else(|| "legacy host closed connection".to_string())?;
            if line.is_empty() {
                continue;
            }
            if line.starts_with("HELLO ") {
                continue;
            }
            last = line.clone();
            if expected_prefix.is_empty() || line.starts_with(expected_prefix) {
                return Ok(line);
            }
        }

        Err(format!(
            "unexpected response for command '{command}', last='{last}'"
        ))
    }

    fn ensure_ready(&mut self) -> Result<(), String> {
        self.connect_if_needed()?;
        if self.activated {
            return Ok(());
        }

        let _ = self.send_command_raw("ACTIVATE", "OK ACTIVATE")?;
        let cp_cmd = format!("CP {}", self.codepage);
        let _ = self.send_command_raw(&cp_cmd, "OK CP")?;
        self.activated = true;
        Ok(())
    }

    fn send_command(&mut self, command: &str, expected_prefix: &str) -> Result<String, String> {
        let mut last_error = "unknown error".to_string();
        for _ in 0..2 {
            if let Err(err) = self.ensure_ready() {
                last_error = err;
                self.reset_connection();
                continue;
            }

            match self.send_command_raw(command, expected_prefix) {
                Ok(line) => return Ok(line),
                Err(err) => {
                    last_error = err;
                    self.reset_connection();
                }
            }
        }
        Err(last_error)
    }

    fn query_remote_update(&mut self, max_candidates: usize) -> Result<(), String> {
        if self.input.is_empty() {
            self.composition.clear();
            self.reading.clear();
            self.candidates.clear();
            return Ok(());
        }

        let mut commands = vec![
            self.command.clone(),
            "CAND".to_string(),
            "KEYTEXTU".to_string(),
            "TEXTU".to_string(),
        ];
        commands.dedup();

        let mut last_comp = self.input.clone();
        let mut last_read = self.input.clone();
        let mut last_candidates: Vec<WorkerIpcCandidate> = Vec::new();

        for cmd in commands {
            let line = self.send_command(&format!("{} {}", cmd, self.input), "CAND_RET ")?;
            let (comp, read, cands) = parse_candidate_reply(&line, max_candidates)?;

            if !comp.is_empty() {
                last_comp = comp;
            }
            if !read.is_empty() {
                last_read = read;
            }
            if !cands.is_empty() {
                last_candidates = cands;
                break;
            }
            last_candidates = cands;
        }

        self.composition = last_comp;
        self.reading = last_read;
        self.candidates = last_candidates;
        Ok(())
    }

    fn reset_for_new_session(&mut self) -> u64 {
        self.backend_state_version += 1;
        self.clear_local_state();
        if self.ensure_ready().is_ok() {
            let _ = self.send_command("RESET", "OK RESET");
        }
        self.backend_state_version
    }

    fn apply_key_event(
        &mut self,
        key_event: &BackendKeyEvent,
        max_candidates: usize,
    ) -> Result<WorkerEventResult, String> {
        self.backend_state_version += 1;

        if key_event.key_down {
            match key_event.virtual_key {
                0x08 => {
                    self.input.pop();
                }
                0x20..=0x7E => {
                    let ch = (key_event.virtual_key as u8 as char).to_ascii_lowercase();
                    if ch.is_ascii_alphanumeric() || ch == '\'' {
                        self.input.push(ch);
                    }
                }
                _ => {}
            }
        }

        self.query_remote_update(max_candidates)?;

        Ok(WorkerEventResult {
            composition: self.composition.clone(),
            backend_state_version: self.backend_state_version,
        })
    }

    fn query_candidates(&mut self, input_snapshot: &str, max_candidates: usize) -> Result<WorkerQueryResult, String> {
        self.backend_state_version += 1;
        if !input_snapshot.is_empty() {
            self.input = input_snapshot.to_string();
        }

        self.query_remote_update(max_candidates)?;

        Ok(WorkerQueryResult {
            composition: self.composition.clone(),
            reading: self.reading.clone(),
            candidates: self
                .candidates
                .clone()
                .into_iter()
                .map(|c| c.into_proto())
                .collect(),
            selected_index: 0,
            page_size: max_candidates as u32,
            backend_state_version: self.backend_state_version,
        })
    }

    fn commit_selection(
        &mut self,
        committed_text: &str,
        candidate_index: usize,
    ) -> Result<WorkerCommitResult, String> {
        let mut committed = committed_text.to_string();
        if committed.is_empty() {
            if let Some(item) = self.candidates.get(candidate_index) {
                committed = item.text.clone();
            }
        }
        if committed.is_empty() {
            return Err("no committed_text and candidate_index is invalid".to_string());
        }

        self.backend_state_version += 1;
        self.last_commit = committed.clone();
        self.clear_local_state();
        if self.ensure_ready().is_ok() {
            let _ = self.send_command("RESET", "OK RESET");
        }

        Ok(WorkerCommitResult {
            committed_text: committed,
            backend_state_version: self.backend_state_version,
        })
    }

    fn reset(&mut self) -> u64 {
        self.reset_for_new_session()
    }
}

impl ImeBackend for LegacyTcpRuntime {
    fn name(&self) -> &'static str {
        "legacy_tcp"
    }

    fn snapshot(&self) -> BackendSnapshot {
        BackendSnapshot {
            composition: self.composition.clone(),
            backend_state_version: self.backend_state_version,
        }
    }

    fn reset_for_new_session(&mut self) -> u64 {
        LegacyTcpRuntime::reset_for_new_session(self)
    }

    fn apply_key_event(
        &mut self,
        key_event: &BackendKeyEvent,
        max_candidates: usize,
    ) -> Result<BackendEventResult, String> {
        let event = LegacyTcpRuntime::apply_key_event(self, key_event, max_candidates)?;
        Ok(BackendEventResult {
            composition: event.composition,
            reading: self.reading.clone(),
            candidates: self
                .candidates
                .clone()
                .into_iter()
                .map(BackendCandidate::from)
                .collect(),
            selected_index: 0,
            page_size: max_candidates as u32,
            backend_state_version: event.backend_state_version,
        })
    }

    fn query_candidates(
        &mut self,
        input_snapshot: &str,
        max_candidates: usize,
    ) -> Result<BackendQueryResult, String> {
        let query = LegacyTcpRuntime::query_candidates(self, input_snapshot, max_candidates)?;
        Ok(BackendQueryResult {
            composition: query.composition,
            reading: query.reading,
            candidates: query
                .candidates
                .into_iter()
                .map(WorkerIpcCandidate::from)
                .map(BackendCandidate::from)
                .collect(),
            selected_index: query.selected_index,
            page_size: query.page_size,
            backend_state_version: query.backend_state_version,
        })
    }

    fn commit_selection(
        &mut self,
        committed_text: &str,
        candidate_index: usize,
    ) -> Result<BackendCommitResult, String> {
        let commit = LegacyTcpRuntime::commit_selection(self, committed_text, candidate_index)?;
        Ok(BackendCommitResult {
            committed_text: commit.committed_text,
            backend_state_version: commit.backend_state_version,
        })
    }

    fn reset(&mut self) -> Result<u64, String> {
        Ok(LegacyTcpRuntime::reset(self))
    }
}

#[derive(Debug)]
enum WorkerBackendRuntime {
    Stub(WorkerRuntime),
    LegacyTcp(LegacyTcpRuntime),
    WinImm(WinImmBackend),
}

impl WorkerBackendRuntime {
    fn from_env() -> Self {
        let mode = env::var("IME_WORKER_BACKEND")
            .unwrap_or_else(|_| "stub".to_string())
            .to_ascii_lowercase();

        match mode.as_str() {
            "win_imm" | "winimm" => {
                info!("worker runtime backend: win_imm");
                Self::WinImm(WinImmBackend::from_env())
            }
            "legacy_tcp" | "legacy" => {
                info!("worker runtime backend: legacy_tcp");
                Self::LegacyTcp(LegacyTcpRuntime::from_env())
            }
            other => {
                if other != "stub" {
                    warn!(backend = %other, "unknown worker backend, fallback to stub");
                } else {
                    info!("worker runtime backend: stub");
                }
                Self::Stub(WorkerRuntime::new())
            }
        }
    }

    fn snapshot(&self) -> WorkerSnapshot {
        let snapshot = match self {
            Self::Stub(runtime) => ImeBackend::snapshot(runtime),
            Self::LegacyTcp(runtime) => ImeBackend::snapshot(runtime),
            Self::WinImm(runtime) => ImeBackend::snapshot(runtime),
        };

        WorkerSnapshot {
            composition: snapshot.composition,
            backend_state_version: snapshot.backend_state_version,
        }
    }

    fn reset_for_new_session(&mut self) -> u64 {
        match self {
            Self::Stub(runtime) => ImeBackend::reset_for_new_session(runtime),
            Self::LegacyTcp(runtime) => ImeBackend::reset_for_new_session(runtime),
            Self::WinImm(runtime) => ImeBackend::reset_for_new_session(runtime),
        }
    }

    fn handle_request(&mut self, req: &WorkerIpcRequest) -> WorkerIpcResponse {
        match self {
            Self::Stub(runtime) => handle_backend_request(runtime, req),
            Self::LegacyTcp(runtime) => handle_backend_request(runtime, req),
            Self::WinImm(runtime) => handle_backend_request(runtime, req),
        }
    }
}

fn handle_backend_request<B: ImeBackend>(backend: &mut B, req: &WorkerIpcRequest) -> WorkerIpcResponse {
    backend.set_debug_timeline_enabled(req.trace_timeline);

    match req.op.as_str() {
        "snapshot" => {
            let snapshot = backend.snapshot();
            WorkerIpcResponse {
                ok: true,
                backend_state_version: snapshot.backend_state_version,
                composition: snapshot.composition,
                reading: String::new(),
                candidates: Vec::new(),
                selected_index: 0,
                page_size: 0,
                committed_text: String::new(),
                error_code: String::new(),
                error_message: String::new(),
                debug_timeline: backend.drain_debug_timeline(),
            }
        }
        "reset_session" => match backend.reset() {
            Ok(version) => WorkerIpcResponse {
                ok: true,
                backend_state_version: version,
                composition: String::new(),
                reading: String::new(),
                candidates: Vec::new(),
                selected_index: 0,
                page_size: 0,
                committed_text: String::new(),
                error_code: String::new(),
                error_message: String::new(),
                debug_timeline: backend.drain_debug_timeline(),
            },
            Err(message) => {
                let mut resp = worker_error_response(
                    WorkerSnapshot {
                        composition: backend.snapshot().composition,
                        backend_state_version: backend.snapshot().backend_state_version,
                    },
                    "BACKEND_RESET_FAILED",
                    &message,
                );
                resp.debug_timeline = backend.drain_debug_timeline();
                resp
            }
        },
        "send_key_event" => {
            let max_candidates = if req.max_candidates == 0 {
                9
            } else {
                req.max_candidates
            };

            let key_event = BackendKeyEvent {
                key_down: req.key_down,
                virtual_key: req.virtual_key,
                scan_code: req.scan_code,
                shift: req.shift,
                ctrl: req.ctrl,
                alt: req.alt,
                repeated: req.repeated,
                extended: req.extended,
                timestamp_ms: req.timestamp_ms,
                source_keycode: req.source_keycode,
                source_modifier: req.source_modifier,
            };

            match backend.apply_key_event(&key_event, max_candidates) {
                Ok(event) => WorkerIpcResponse {
                    ok: true,
                    backend_state_version: event.backend_state_version,
                    composition: event.composition,
                    reading: event.reading,
                    candidates: event.candidates.into_iter().map(WorkerIpcCandidate::from).collect(),
                    selected_index: event.selected_index,
                    page_size: event.page_size,
                    committed_text: String::new(),
                    error_code: String::new(),
                    error_message: String::new(),
                    debug_timeline: backend.drain_debug_timeline(),
                },
                Err(message) => {
                    let mut resp = worker_error_response(
                        WorkerSnapshot {
                            composition: backend.snapshot().composition,
                            backend_state_version: backend.snapshot().backend_state_version,
                        },
                        "BACKEND_SEND_KEY_FAILED",
                        &message,
                    );
                    resp.debug_timeline = backend.drain_debug_timeline();
                    resp
                }
            }
        }
        "query_candidates" => {
            let max_candidates = if req.max_candidates == 0 {
                9
            } else {
                req.max_candidates
            };

            match backend.query_candidates(&req.input_snapshot, max_candidates) {
                Ok(query) => WorkerIpcResponse {
                    ok: true,
                    backend_state_version: query.backend_state_version,
                    composition: query.composition,
                    reading: query.reading,
                    candidates: query.candidates.into_iter().map(WorkerIpcCandidate::from).collect(),
                    selected_index: query.selected_index,
                    page_size: query.page_size,
                    committed_text: String::new(),
                    error_code: String::new(),
                    error_message: String::new(),
                    debug_timeline: backend.drain_debug_timeline(),
                },
                Err(message) => {
                    let mut resp = worker_error_response(
                        WorkerSnapshot {
                            composition: backend.snapshot().composition,
                            backend_state_version: backend.snapshot().backend_state_version,
                        },
                        "BACKEND_QUERY_FAILED",
                        &message,
                    );
                    resp.debug_timeline = backend.drain_debug_timeline();
                    resp
                }
            }
        }
        "commit_selection" => match backend.commit_selection(&req.committed_text, req.candidate_index) {
            Ok(result) => WorkerIpcResponse {
                ok: true,
                backend_state_version: result.backend_state_version,
                composition: String::new(),
                reading: String::new(),
                candidates: Vec::new(),
                selected_index: 0,
                page_size: 0,
                committed_text: result.committed_text,
                error_code: String::new(),
                error_message: String::new(),
                debug_timeline: backend.drain_debug_timeline(),
            },
            Err(message) => {
                let mut resp = worker_error_response(
                    WorkerSnapshot {
                        composition: backend.snapshot().composition,
                        backend_state_version: backend.snapshot().backend_state_version,
                    },
                    "BACKEND_COMMIT_FAILED",
                    &message,
                );
                resp.debug_timeline = backend.drain_debug_timeline();
                resp
            }
        },
        other => {
            let mut resp = worker_error_response(
                WorkerSnapshot {
                    composition: backend.snapshot().composition,
                    backend_state_version: backend.snapshot().backend_state_version,
                },
                "UNSUPPORTED_OP",
                &format!("unsupported op: {other} for backend {}", backend.name()),
            );
            resp.debug_timeline = backend.drain_debug_timeline();
            resp
        }
    }
}

fn run_worker_runtime_mode() -> Result<(), Box<dyn std::error::Error>> {
    let enable_worker_runtime_log = env_bool("IME_WORKER_RUNTIME_LOG", false)
        || env_bool("IME_WINIMM_TRACE_TIMELINE", false)
        || env::var("RUST_LOG").is_ok();
    if enable_worker_runtime_log {
        init_tracing(true);
    }

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    let mut runtime = WorkerBackendRuntime::from_env();
    let _ = runtime.reset_for_new_session();

    loop {
        let payload = match read_framed_payload(&mut reader)? {
            Some(payload) => payload,
            None => break,
        };

        let req = match serde_json::from_slice::<WorkerIpcRequest>(&payload) {
            Ok(req) => req,
            Err(err) => {
                let payload = serde_json::to_vec(&worker_error_response(
                    runtime.snapshot(),
                    "BAD_REQUEST",
                    &format!("invalid request json: {err}"),
                ))?;
                write_framed_payload(&mut writer, &payload)?;
                continue;
            }
        };

        let resp = runtime.handle_request(&req);

        let payload = serde_json::to_vec(&resp)?;
        write_framed_payload(&mut writer, &payload)?;
    }

    Ok(())
}

#[derive(Debug)]
struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl WorkerProcess {
    fn request(&mut self, req: &WorkerIpcRequest) -> Result<WorkerIpcResponse, String> {
        let payload =
            serde_json::to_vec(req).map_err(|e| format!("serialize request failed: {e}"))?;

        write_framed_payload(&mut self.stdin, &payload)
            .map_err(|e| format!("write request failed: {e}"))?;

        let payload = read_framed_payload(&mut self.stdout)
            .map_err(|e| format!("read response failed: {e}"))?
            .ok_or_else(|| "worker closed pipe unexpectedly".to_string())?;

        serde_json::from_slice::<WorkerIpcResponse>(&payload)
            .map_err(|e| format!("parse response failed: {e}"))
    }

    fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Clone, Debug)]
struct WorkerEventResult {
    composition: String,
    backend_state_version: u64,
}

#[derive(Clone, Debug)]
struct WorkerQueryResult {
    composition: String,
    reading: String,
    candidates: Vec<CandidateItem>,
    selected_index: u32,
    page_size: u32,
    backend_state_version: u64,
}

#[derive(Clone, Debug)]
struct WorkerCommitResult {
    committed_text: String,
    backend_state_version: u64,
}

#[derive(Clone, Debug)]
struct WorkerSnapshot {
    composition: String,
    backend_state_version: u64,
}

#[derive(Debug)]
struct WorkerRuntime {
    input: String,
    reading: String,
    candidates: Vec<WorkerIpcCandidate>,
    last_commit: String,
    backend_state_version: u64,
}

impl WorkerRuntime {
    fn new() -> Self {
        Self {
            input: String::new(),
            reading: String::new(),
            candidates: Vec::new(),
            last_commit: String::new(),
            backend_state_version: 1,
        }
    }

    fn reset_for_new_session(&mut self) -> u64 {
        self.input.clear();
        self.reading.clear();
        self.candidates.clear();
        self.last_commit.clear();
        self.backend_state_version += 1;
        self.backend_state_version
    }

    fn fake_candidates(input: &str, max_count: usize) -> Vec<WorkerIpcCandidate> {
        if input.is_empty() || max_count == 0 {
            return Vec::new();
        }

        let mut templates = vec![
            input.to_string(),
            format!("{}1", input),
            format!("{}2", input),
            format!("{}3", input),
            format!("{}4", input),
        ];
        templates.dedup();

        templates
            .into_iter()
            .take(max_count)
            .enumerate()
            .map(|(idx, text)| WorkerIpcCandidate {
                index: idx as u32,
                text,
                comment: "stub".to_string(),
                quality: (100.0 - idx as f64).max(1.0),
            })
            .collect()
    }

    fn apply_key_event(
        &mut self,
        key_event: &BackendKeyEvent,
        max_candidates: usize,
    ) -> WorkerEventResult {
        self.backend_state_version += 1;

        if key_event.key_down {
            match key_event.virtual_key {
                0x08 => {
                    self.input.pop();
                }
                0x20..=0x7E => {
                    let ch = (key_event.virtual_key as u8 as char).to_ascii_lowercase();
                    if ch.is_ascii_alphanumeric() || ch == '\'' {
                        self.input.push(ch);
                    }
                }
                _ => {}
            }
        }

        self.reading = self.input.clone();
        self.candidates = Self::fake_candidates(&self.input, max_candidates);

        WorkerEventResult {
            composition: self.input.clone(),
            backend_state_version: self.backend_state_version,
        }
    }

    fn query_candidates(&mut self, input_snapshot: &str, max_candidates: usize) -> WorkerQueryResult {
        self.backend_state_version += 1;

        if !input_snapshot.is_empty() {
            self.input = input_snapshot.to_string();
        }

        self.reading = self.input.clone();
        self.candidates = Self::fake_candidates(&self.input, max_candidates);

        WorkerQueryResult {
            composition: self.input.clone(),
            reading: self.reading.clone(),
            candidates: self
                .candidates
                .clone()
                .into_iter()
                .map(|c| c.into_proto())
                .collect(),
            selected_index: 0,
            page_size: max_candidates as u32,
            backend_state_version: self.backend_state_version,
        }
    }

    fn commit_selection(
        &mut self,
        committed_text: &str,
        candidate_index: usize,
    ) -> Result<WorkerCommitResult, &'static str> {
        let mut committed = committed_text.to_string();
        if committed.is_empty() {
            if let Some(item) = self.candidates.get(candidate_index) {
                committed = item.text.clone();
            }
        }

        if committed.is_empty() {
            return Err("no committed_text and candidate_index is invalid");
        }

        self.backend_state_version += 1;
        self.last_commit = committed.clone();
        self.input.clear();
        self.reading.clear();
        self.candidates.clear();

        Ok(WorkerCommitResult {
            committed_text: committed,
            backend_state_version: self.backend_state_version,
        })
    }

    fn reset(&mut self) -> u64 {
        self.backend_state_version += 1;
        self.input.clear();
        self.reading.clear();
        self.candidates.clear();
        self.last_commit.clear();
        self.backend_state_version
    }

}

impl ImeBackend for WorkerRuntime {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn snapshot(&self) -> BackendSnapshot {
        BackendSnapshot {
            composition: self.input.clone(),
            backend_state_version: self.backend_state_version,
        }
    }

    fn reset_for_new_session(&mut self) -> u64 {
        WorkerRuntime::reset_for_new_session(self)
    }

    fn apply_key_event(
        &mut self,
        key_event: &BackendKeyEvent,
        max_candidates: usize,
    ) -> Result<BackendEventResult, String> {
        let event = WorkerRuntime::apply_key_event(self, key_event, max_candidates);
        Ok(BackendEventResult {
            composition: event.composition,
            reading: self.reading.clone(),
            candidates: self
                .candidates
                .clone()
                .into_iter()
                .map(BackendCandidate::from)
                .collect(),
            selected_index: 0,
            page_size: max_candidates as u32,
            backend_state_version: event.backend_state_version,
        })
    }

    fn query_candidates(
        &mut self,
        input_snapshot: &str,
        max_candidates: usize,
    ) -> Result<BackendQueryResult, String> {
        let query = WorkerRuntime::query_candidates(self, input_snapshot, max_candidates);
        Ok(BackendQueryResult {
            composition: query.composition,
            reading: query.reading,
            candidates: query
                .candidates
                .into_iter()
                .map(WorkerIpcCandidate::from)
                .map(BackendCandidate::from)
                .collect(),
            selected_index: query.selected_index,
            page_size: query.page_size,
            backend_state_version: query.backend_state_version,
        })
    }

    fn commit_selection(
        &mut self,
        committed_text: &str,
        candidate_index: usize,
    ) -> Result<BackendCommitResult, String> {
        let commit = WorkerRuntime::commit_selection(self, committed_text, candidate_index)
            .map_err(|e| e.to_string())?;
        Ok(BackendCommitResult {
            committed_text: commit.committed_text,
            backend_state_version: commit.backend_state_version,
        })
    }

    fn reset(&mut self) -> Result<u64, String> {
        Ok(WorkerRuntime::reset(self))
    }
}

#[derive(Debug)]
struct WorkerPool {
    cfg: PoolConfig,
    next_worker_id: u64,
    idle: VecDeque<String>,
    busy_by_session: HashMap<String, String>,
    workers: HashMap<String, WorkerProcess>,
}

fn emit_worker_timeline(worker_id: &str, timeline: &[String]) {
    if timeline.is_empty() {
        return;
    }

    for marker in timeline {
        info!(target: "win_imm_timeline", worker_id = %worker_id, marker = %marker);
    }
}

impl WorkerPool {
    fn new(cfg: PoolConfig) -> Self {
        let mut pool = Self {
            cfg,
            next_worker_id: 0,
            idle: VecDeque::new(),
            busy_by_session: HashMap::new(),
            workers: HashMap::new(),
        };
        if pool.cfg.prewarm {
            pool.prewarm();
        }
        pool
    }

    fn allocate_worker_id(&mut self) -> String {
        self.next_worker_id += 1;
        format!("worker-{:06}", self.next_worker_id)
    }

    fn spawn_worker(&mut self, warm: bool) -> Option<WorkerHandle> {
        let worker_id = self.allocate_worker_id();

        let exe_path = match env::current_exe() {
            Ok(path) => path,
            Err(err) => {
                tracing::warn!("cannot resolve current_exe for worker spawn: {err}");
                return None;
            }
        };

        let mut child = match Command::new(exe_path)
            .arg("--worker-runtime")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                tracing::warn!(worker_id = %worker_id, "spawn worker process failed: {err}");
                return None;
            }
        };

        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(worker_id = %worker_id, "worker stdin is not piped");
                return None;
            }
        };

        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(worker_id = %worker_id, "worker stdout is not piped");
                return None;
            }
        };

        let mut process = WorkerProcess {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };

        match process.request(&WorkerIpcRequest::reset_session()) {
            Ok(resp) => {
                emit_worker_timeline(&worker_id, &resp.debug_timeline);
                if resp.ok {
                    self.workers.insert(worker_id.clone(), process);
                    info!(
                        worker_id = %worker_id,
                        warm,
                        spawn_timeout_ms = self.cfg.spawn_timeout_ms,
                        "spawned worker subprocess"
                    );
                    Some(WorkerHandle { worker_id })
                } else {
                    process.shutdown();
                    tracing::warn!(
                        worker_id = %worker_id,
                        error_code = %resp.error_code,
                        error_message = %resp.error_message,
                        "worker reset_session returned backend error"
                    );
                    None
                }
            }
            Err(err) => {
                process.shutdown();
                tracing::warn!(worker_id = %worker_id, "worker reset_session failed: {err}");
                None
            }
        }
    }

    fn prewarm(&mut self) {
        while self.idle.len() < self.cfg.min_idle {
            match self.spawn_worker(true) {
                Some(worker) => self.idle.push_back(worker.worker_id),
                None => break,
            }
        }
    }

    fn acquire_for_session(
        &mut self,
        session_id: &str,
        want_prewarmed: bool,
    ) -> Result<(String, u64), String> {
        let worker_id = if want_prewarmed {
            self.idle
                .pop_front()
                .or_else(|| self.spawn_worker(false).map(|w| w.worker_id))
                .ok_or_else(|| "failed to acquire prewarmed worker".to_string())?
        } else {
            self.spawn_worker(false)
                .map(|w| w.worker_id)
                .ok_or_else(|| "failed to spawn dedicated worker".to_string())?
        };

        let version = match self.workers.get_mut(&worker_id) {
            Some(worker) => match worker.request(&WorkerIpcRequest::reset_session()) {
                Ok(resp) => {
                    emit_worker_timeline(&worker_id, &resp.debug_timeline);
                    if resp.ok {
                        resp.backend_state_version
                    } else {
                        return Err(format!(
                            "worker reset failed: code={} msg={}",
                            resp.error_code, resp.error_message
                        ));
                    }
                }
                Err(err) => {
                    return Err(format!("worker reset transport failed: {err}"));
                }
            },
            None => {
                return Err("worker disappeared from registry".to_string());
            }
        };

        self.busy_by_session
            .insert(session_id.to_string(), worker_id.clone());

        Ok((worker_id, version))
    }

    fn release_for_session(&mut self, session_id: &str) {
        if let Some(worker_id) = self.busy_by_session.remove(session_id) {
            if self.idle.len() < self.cfg.max_idle {
                if let Some(worker) = self.workers.get_mut(&worker_id) {
                    if let Ok(resp) = worker.request(&WorkerIpcRequest::reset_session()) {
                        emit_worker_timeline(&worker_id, &resp.debug_timeline);
                    }
                }
                self.idle.push_back(worker_id);
            } else {
                if let Some(mut worker) = self.workers.remove(&worker_id) {
                    worker.shutdown();
                }
            }
        }
    }

    fn trim_idle(&mut self) {
        while self.idle.len() > self.cfg.max_idle {
            if let Some(worker_id) = self.idle.pop_back() {
                if let Some(mut worker) = self.workers.remove(&worker_id) {
                    worker.shutdown();
                }
            }
        }
    }

    fn snapshot(&mut self, worker_id: &str) -> Result<WorkerSnapshot, String> {
        let worker = self
            .workers
            .get_mut(worker_id)
            .ok_or_else(|| "worker process missing".to_string())?;
        let resp = worker
            .request(&WorkerIpcRequest::snapshot())
            .map_err(|e| format!("worker snapshot transport failed: {e}"))?;
        emit_worker_timeline(worker_id, &resp.debug_timeline);
        if !resp.ok {
            return Err(format!(
                "worker snapshot backend failed: code={} msg={}",
                resp.error_code, resp.error_message
            ));
        }

        Ok(WorkerSnapshot {
            composition: resp.composition,
            backend_state_version: resp.backend_state_version,
        })
    }

    fn apply_key_event(
        &mut self,
        worker_id: &str,
        key_event: &BackendKeyEvent,
        max_candidates: usize,
    ) -> Result<WorkerEventResult, String> {
        let worker = self
            .workers
            .get_mut(worker_id)
            .ok_or_else(|| "worker process missing".to_string())?;
        let resp = worker
            .request(&WorkerIpcRequest::send_key_event(key_event, max_candidates))
            .map_err(|e| format!("worker send_key_event transport failed: {e}"))?;
        emit_worker_timeline(worker_id, &resp.debug_timeline);
        if !resp.ok {
            return Err(format!(
                "worker send_key_event backend failed: code={} msg={}",
                resp.error_code, resp.error_message
            ));
        }

        Ok(WorkerEventResult {
            composition: resp.composition,
            backend_state_version: resp.backend_state_version,
        })
    }

    fn query_candidates(
        &mut self,
        worker_id: &str,
        input_snapshot: &str,
        max_candidates: usize,
    ) -> Result<WorkerQueryResult, String> {
        let worker = self
            .workers
            .get_mut(worker_id)
            .ok_or_else(|| "worker process missing".to_string())?;
        let resp = worker
            .request(&WorkerIpcRequest::query_candidates(
                input_snapshot.to_string(),
                max_candidates,
            ))
            .map_err(|e| format!("worker query_candidates transport failed: {e}"))?;
        emit_worker_timeline(worker_id, &resp.debug_timeline);
        if !resp.ok {
            return Err(format!(
                "worker query_candidates backend failed: code={} msg={}",
                resp.error_code, resp.error_message
            ));
        }

        Ok(WorkerQueryResult {
            composition: resp.composition,
            reading: resp.reading,
            candidates: resp.candidates.into_iter().map(|c| c.into_proto()).collect(),
            selected_index: resp.selected_index,
            page_size: resp.page_size,
            backend_state_version: resp.backend_state_version,
        })
    }

    fn commit_selection(
        &mut self,
        worker_id: &str,
        committed_text: &str,
        candidate_index: usize,
    ) -> Result<WorkerCommitResult, String> {
        let worker = self
            .workers
            .get_mut(worker_id)
            .ok_or_else(|| "worker process missing".to_string())?;
        let resp = worker
            .request(&WorkerIpcRequest::commit_selection(
                committed_text.to_string(),
                candidate_index,
            ))
            .map_err(|e| format!("worker commit_selection transport failed: {e}"))?;
        emit_worker_timeline(worker_id, &resp.debug_timeline);

        if !resp.ok {
            let error_code = if resp.error_code.is_empty() {
                "UNKNOWN".to_string()
            } else {
                resp.error_code
            };
            let error_message = if resp.error_message.is_empty() {
                "worker commit failed".to_string()
            } else {
                resp.error_message
            };
            return Err(format!(
                "worker commit backend failed: code={} msg={}",
                error_code, error_message
            ));
        }

        Ok(WorkerCommitResult {
            committed_text: resp.committed_text,
            backend_state_version: resp.backend_state_version,
        })
    }

    fn reset_worker(&mut self, worker_id: &str) -> Result<u64, String> {
        let worker = self
            .workers
            .get_mut(worker_id)
            .ok_or_else(|| "worker process missing".to_string())?;
        let resp = worker
            .request(&WorkerIpcRequest::reset_session())
            .map_err(|e| format!("worker reset transport failed: {e}"))?;
        emit_worker_timeline(worker_id, &resp.debug_timeline);
        if !resp.ok {
            return Err(format!(
                "worker reset backend failed: code={} msg={}",
                resp.error_code, resp.error_message
            ));
        }
        Ok(resp.backend_state_version)
    }

    fn idle_count(&self) -> usize {
        self.idle.len()
    }

    fn busy_count(&self) -> usize {
        self.busy_by_session.len()
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        for (_, mut worker) in self.workers.drain() {
            worker.shutdown();
        }
    }
}

#[derive(Clone, Debug)]
struct SessionState {
    worker_id: String,
    last_seq: u64,
    last_active: Instant,
}

impl SessionState {
    fn new(worker_id: String) -> Self {
        Self {
            worker_id,
            last_seq: 0,
            last_active: Instant::now(),
        }
    }

    fn touch(&mut self) {
        self.last_active = Instant::now();
    }
}

#[derive(Debug)]
struct GatewayState {
    sessions: RwLock<HashMap<String, SessionState>>,
    pool: Mutex<WorkerPool>,
    session_idle_ttl: Duration,
}

#[derive(Clone)]
struct ImeGatewayService {
    state: Arc<GatewayState>,
}

impl ImeGatewayService {
    fn new(state: Arc<GatewayState>) -> Self {
        Self { state }
    }

    fn now_ms() -> i64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_millis() as i64,
            Err(_) => 0,
        }
    }
}

#[tonic::async_trait]
impl ImeGateway for ImeGatewayService {
    async fn open_session(
        &self,
        request: Request<OpenSessionRequest>,
    ) -> Result<Response<OpenSessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = Uuid::new_v4().to_string();

        let (worker_id, worker_state_version, idle_after_assign) = {
            let mut pool = self.state.pool.lock().await;
            let (worker_id, worker_state_version) = pool
                .acquire_for_session(&session_id, req.want_prewarmed_worker)
                .map_err(Status::internal)?;
            (
                worker_id,
                worker_state_version,
                pool.idle_count() as u32,
            )
        };

        {
            let mut sessions = self.state.sessions.write().await;
            sessions.insert(session_id.clone(), SessionState::new(worker_id.clone()));
        }

        info!(
            session_id = %session_id,
            worker_id = %worker_id,
            frontend_id = %req.frontend_id,
            schema_id = %req.schema_id,
            want_prewarmed_worker = req.want_prewarmed_worker,
            "session opened"
        );

        Ok(Response::new(OpenSessionResponse {
            session_id,
            worker_id,
            backend_state_version: worker_state_version,
            idle_workers_after_assign: idle_after_assign,
        }))
    }

    async fn send_key_event(
        &self,
        request: Request<SendKeyEventRequest>,
    ) -> Result<Response<SendKeyEventResponse>, Status> {
        let req = request.into_inner();
        let event = req
            .key_event
            .ok_or_else(|| Status::invalid_argument("missing key_event"))?;

        let (worker_id, last_seq) = {
            let sessions = self.state.sessions.read().await;
            let session = sessions
                .get(&req.session_id)
                .ok_or_else(|| Status::not_found("session not found"))?;
            (session.worker_id.clone(), session.last_seq)
        };

        if event.seq <= last_seq {
            let snapshot = {
                let mut pool = self.state.pool.lock().await;
                match pool.snapshot(&worker_id) {
                    Ok(snapshot) => snapshot,
                    Err(error_message) => {
                        warn!(
                            session_id = %req.session_id,
                            worker_id = %worker_id,
                            error = %error_message,
                            "failed to snapshot worker on seq check"
                        );
                        return Ok(Response::new(SendKeyEventResponse {
                            session_id: req.session_id.clone(),
                            acknowledged_seq: last_seq,
                            backend_state_version: 0,
                            composition: String::new(),
                            error_code: "BACKEND_SNAPSHOT_FAILED".to_string(),
                            error_message,
                        }));
                    }
                }
            };

            return Ok(Response::new(SendKeyEventResponse {
                session_id: req.session_id,
                acknowledged_seq: last_seq,
                backend_state_version: snapshot.backend_state_version,
                composition: snapshot.composition,
                error_code: "SEQ_OUT_OF_ORDER".to_string(),
                error_message: "incoming seq is not greater than previous seq".to_string(),
            }));
        }

        {
            let mut sessions = self.state.sessions.write().await;
            let session = sessions
                .get_mut(&req.session_id)
                .ok_or_else(|| Status::not_found("session not found"))?;
            session.last_seq = event.seq;
            session.touch();
        }

        let worker_result = {
            let mut pool = self.state.pool.lock().await;
            let backend_event = BackendKeyEvent {
                key_down: event.key_down,
                virtual_key: event.virtual_key,
                scan_code: event.scan_code,
                shift: event.shift,
                ctrl: event.ctrl,
                alt: event.alt,
                repeated: event.repeated,
                extended: event.extended,
                timestamp_ms: event.timestamp_ms,
                source_keycode: event.source_keycode,
                source_modifier: event.source_modifier,
            };

            match pool.apply_key_event(&worker_id, &backend_event, 9) {
                Ok(result) => result,
                Err(error_message) => {
                    let (backend_state_version, composition) = match pool.snapshot(&worker_id) {
                        Ok(snapshot) => {
                            (snapshot.backend_state_version, snapshot.composition)
                        }
                        Err(snapshot_error) => {
                            warn!(
                                session_id = %req.session_id,
                                worker_id = %worker_id,
                                error = %snapshot_error,
                                "failed to snapshot worker after send_key_event error"
                            );
                            (0, String::new())
                        }
                    };

                    return Ok(Response::new(SendKeyEventResponse {
                        session_id: req.session_id.clone(),
                        acknowledged_seq: event.seq,
                        backend_state_version,
                        composition,
                        error_code: "BACKEND_SEND_KEY_FAILED".to_string(),
                        error_message,
                    }));
                }
            }
        };

        Ok(Response::new(SendKeyEventResponse {
            session_id: req.session_id,
            acknowledged_seq: event.seq,
            backend_state_version: worker_result.backend_state_version,
            composition: worker_result.composition,
            error_code: String::new(),
            error_message: String::new(),
        }))
    }

    async fn query_candidates(
        &self,
        request: Request<QueryCandidatesRequest>,
    ) -> Result<Response<QueryCandidatesResponse>, Status> {
        let req = request.into_inner();
        let max = if req.max_candidates == 0 {
            9
        } else {
            req.max_candidates as usize
        };

        let worker_id = {
            let mut sessions = self.state.sessions.write().await;
            let session = sessions
                .get_mut(&req.session_id)
                .ok_or_else(|| Status::not_found("session not found"))?;

            if req.seq > 0 {
                session.last_seq = session.last_seq.max(req.seq);
            }
            session.touch();
            session.worker_id.clone()
        };

        let worker_result = {
            let mut pool = self.state.pool.lock().await;
            match pool.query_candidates(&worker_id, &req.input_snapshot, max) {
                Ok(result) => result,
                Err(error_message) => {
                    let (backend_state_version, composition) = match pool.snapshot(&worker_id) {
                        Ok(snapshot) => {
                            (snapshot.backend_state_version, snapshot.composition)
                        }
                        Err(snapshot_error) => {
                            warn!(
                                session_id = %req.session_id,
                                worker_id = %worker_id,
                                error = %snapshot_error,
                                "failed to snapshot worker after query_candidates error"
                            );
                            (0, String::new())
                        }
                    };

                    return Ok(Response::new(QueryCandidatesResponse {
                        session_id: req.session_id.clone(),
                        backend_state_version,
                        composition,
                        reading: String::new(),
                        candidates: Vec::new(),
                        selected_index: 0,
                        page_size: 0,
                        error_code: "BACKEND_QUERY_FAILED".to_string(),
                        error_message,
                    }));
                }
            }
        };

        Ok(Response::new(QueryCandidatesResponse {
            session_id: req.session_id,
            backend_state_version: worker_result.backend_state_version,
            composition: worker_result.composition,
            reading: worker_result.reading,
            candidates: worker_result.candidates,
            selected_index: worker_result.selected_index,
            page_size: worker_result.page_size,
            error_code: String::new(),
            error_message: String::new(),
        }))
    }

    async fn commit_selection(
        &self,
        request: Request<CommitSelectionRequest>,
    ) -> Result<Response<CommitSelectionResponse>, Status> {
        let req = request.into_inner();

        let worker_id = {
            let mut sessions = self.state.sessions.write().await;
            let session = sessions
                .get_mut(&req.session_id)
                .ok_or_else(|| Status::not_found("session not found"))?;
            session.last_seq = session.last_seq.max(req.seq);
            session.touch();
            session.worker_id.clone()
        };

        let commit_result = {
            let mut pool = self.state.pool.lock().await;
            match pool.commit_selection(
                &worker_id,
                &req.committed_text,
                req.candidate_index as usize,
            ) {
                Ok(result) => result,
                Err(error_message) => {
                    let backend_state_version = match pool.snapshot(&worker_id) {
                        Ok(snapshot) => snapshot.backend_state_version,
                        Err(snapshot_error) => {
                            warn!(
                                session_id = %req.session_id,
                                worker_id = %worker_id,
                                error = %snapshot_error,
                                "failed to snapshot worker after commit_selection error"
                            );
                            0
                        }
                    };

                    return Ok(Response::new(CommitSelectionResponse {
                        session_id: req.session_id.clone(),
                        backend_state_version,
                        committed_text: String::new(),
                        error_code: "BACKEND_COMMIT_FAILED".to_string(),
                        error_message,
                    }));
                }
            }
        };

        Ok(Response::new(CommitSelectionResponse {
            session_id: req.session_id,
            backend_state_version: commit_result.backend_state_version,
            committed_text: commit_result.committed_text,
            error_code: String::new(),
            error_message: String::new(),
        }))
    }

    async fn reset_session(
        &self,
        request: Request<ResetSessionRequest>,
    ) -> Result<Response<ResetSessionResponse>, Status> {
        let req = request.into_inner();

        let worker_id = {
            let mut sessions = self.state.sessions.write().await;
            let session = sessions
                .get_mut(&req.session_id)
                .ok_or_else(|| Status::not_found("session not found"))?;
            session.touch();
            session.worker_id.clone()
        };

        let backend_state_version = {
            let mut pool = self.state.pool.lock().await;
            match pool.reset_worker(&worker_id) {
                Ok(version) => version,
                Err(error_message) => {
                    warn!(
                        session_id = %req.session_id,
                        worker_id = %worker_id,
                        error = %error_message,
                        "reset_session failed"
                    );
                    return Ok(Response::new(ResetSessionResponse {
                        session_id: req.session_id.clone(),
                        backend_state_version: 0,
                        ok: false,
                        error_code: "BACKEND_RESET_FAILED".to_string(),
                        error_message,
                    }));
                }
            }
        };

        Ok(Response::new(ResetSessionResponse {
            session_id: req.session_id,
            backend_state_version,
            ok: true,
            error_code: String::new(),
            error_message: String::new(),
        }))
    }

    async fn get_status(
        &self,
        _request: Request<GetStatusRequest>,
    ) -> Result<Response<GetStatusResponse>, Status> {
        let active_sessions = {
            let sessions = self.state.sessions.read().await;
            sessions.len() as u32
        };

        let (idle_workers, busy_workers) = {
            let pool = self.state.pool.lock().await;
            (pool.idle_count() as u32, pool.busy_count() as u32)
        };

        Ok(Response::new(GetStatusResponse {
            ok: true,
            active_sessions,
            idle_workers,
            busy_workers,
            message: "ok".to_string(),
        }))
    }

    async fn ping(&self, request: Request<PingRequest>) -> Result<Response<PingResponse>, Status> {
        let req = request.into_inner();
        Ok(Response::new(PingResponse {
            payload: req.payload,
            server_unix_ms: Self::now_ms(),
        }))
    }
}

async fn reap_idle_sessions(state: Arc<GatewayState>) {
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;

        let now = Instant::now();
        let ttl = state.session_idle_ttl;

        let expired_session_ids: Vec<String> = {
            let sessions = state.sessions.read().await;
            sessions
                .iter()
                .filter_map(|(session_id, session)| {
                    if now.duration_since(session.last_active) > ttl {
                        Some(session_id.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        if expired_session_ids.is_empty() {
            continue;
        }

        {
            let mut sessions = state.sessions.write().await;
            for session_id in &expired_session_ids {
                if let Some(removed) = sessions.remove(session_id) {
                    info!(
                        session_id = %session_id,
                        worker_id = %removed.worker_id,
                        worker_age_ms = removed.last_active.elapsed().as_millis() as u64,
                        "session expired and removed"
                    );
                }
            }
        }

        {
            let mut pool = state.pool.lock().await;
            for session_id in &expired_session_ids {
                pool.release_for_session(session_id);
            }
            pool.trim_idle();
            pool.prewarm();
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if env::args().nth(1).as_deref() == Some("--worker-runtime") {
        return run_worker_runtime_mode();
    }

    init_tracing(false);

    let bind = env::var("IME_GRPC_BIND").unwrap_or_else(|_| "127.0.0.1:50051".to_string());
    let addr: SocketAddr = bind.parse()?;

    let session_idle_ttl_secs = env_u64("IME_SESSION_IDLE_TTL_SECS", 30);
    let pool_cfg = PoolConfig::from_env();

    info!(
        bind = %bind,
        session_idle_ttl_secs,
        pool_min_idle = pool_cfg.min_idle,
        pool_max_idle = pool_cfg.max_idle,
        pool_prewarm = pool_cfg.prewarm,
        "starting ime-grpc-host"
    );

    let state = Arc::new(GatewayState {
        sessions: RwLock::new(HashMap::new()),
        pool: Mutex::new(WorkerPool::new(pool_cfg)),
        session_idle_ttl: Duration::from_secs(session_idle_ttl_secs),
    });

    let reaper_state = Arc::clone(&state);
    tokio::spawn(async move {
        reap_idle_sessions(reaper_state).await;
    });

    let service = ImeGatewayService::new(state);

    Server::builder()
        .add_service(ImeGatewayServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}

fn env_usize(name: &str, fallback: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(fallback)
}

fn init_tracing(to_stderr: bool) {
    let env_filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "ime_grpc_host=info,info".into());

    if to_stderr {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }
}

fn env_u64(name: &str, fallback: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(fallback)
}

fn env_bool(name: &str, fallback: bool) -> bool {
    env::var(name)
        .ok()
        .and_then(|v| match v.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(fallback)
}
