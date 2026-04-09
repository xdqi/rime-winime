#[allow(unsafe_code)]
pub mod win_imm;

#[derive(Clone, Debug, Default)]
pub struct BackendCandidate {
    pub index: u32,
    pub text: String,
    pub comment: String,
    pub quality: f64,
}

#[derive(Clone, Debug, Default)]
pub struct BackendSnapshot {
    pub composition: String,
    pub backend_state_version: u64,
}

#[derive(Clone, Debug, Default)]
pub struct BackendEventResult {
    pub composition: String,
    pub reading: String,
    pub candidates: Vec<BackendCandidate>,
    pub selected_index: u32,
    pub page_size: u32,
    pub backend_state_version: u64,
}

#[derive(Clone, Debug, Default)]
pub struct BackendQueryResult {
    pub composition: String,
    pub reading: String,
    pub candidates: Vec<BackendCandidate>,
    pub selected_index: u32,
    pub page_size: u32,
    pub backend_state_version: u64,
}

#[derive(Clone, Debug, Default)]
pub struct BackendCommitResult {
    pub committed_text: String,
    pub backend_state_version: u64,
}

#[derive(Clone, Debug, Default)]
pub struct BackendKeyEvent {
    pub key_down: bool,
    pub virtual_key: u32,
    pub scan_code: u32,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub repeated: bool,
    pub extended: bool,
    pub timestamp_ms: i64,
    pub source_keycode: u32,
    pub source_modifier: u32,
}

pub trait ImeBackend: Send {
    fn name(&self) -> &'static str;

    fn snapshot(&self) -> BackendSnapshot;

    fn reset_for_new_session(&mut self) -> u64;

    fn apply_key_event(
        &mut self,
        key_event: &BackendKeyEvent,
        max_candidates: usize,
    ) -> Result<BackendEventResult, String>;

    fn query_candidates(
        &mut self,
        input_snapshot: &str,
        max_candidates: usize,
    ) -> Result<BackendQueryResult, String>;

    fn commit_selection(
        &mut self,
        committed_text: &str,
        candidate_index: usize,
    ) -> Result<BackendCommitResult, String>;

    fn reset(&mut self) -> Result<u64, String> {
        Ok(self.reset_for_new_session())
    }

    fn set_debug_timeline_enabled(&mut self, _enabled: bool) {}

    fn drain_debug_timeline(&mut self) -> Vec<String> {
        Vec::new()
    }
}
