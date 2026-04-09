use super::{
    BackendCandidate, BackendCommitResult, BackendEventResult, BackendKeyEvent,
    BackendQueryResult, BackendSnapshot, ImeBackend,
};

#[derive(Debug, Default)]
pub struct WinImmBackend {}

impl WinImmBackend {
    pub fn from_env() -> Self {
        Self {}
    }
    
    fn not_ready_message(&self) -> String {
        "win_imm backend requires Windows runtime; current build is non-windows".to_string()
    }
}

impl ImeBackend for WinImmBackend {
    fn name(&self) -> &'static str {
        "win_imm"
    }

    fn snapshot(&self) -> BackendSnapshot {
        BackendSnapshot::default()
    }

    fn reset_for_new_session(&mut self) -> u64 {
        0
    }

    fn apply_key_event(
        &mut self,
        _key_event: &BackendKeyEvent,
        _max_candidates: usize,
    ) -> Result<BackendEventResult, String> {
        Err(self.not_ready_message())
    }

    fn query_candidates(
        &mut self,
        _input_snapshot: &str,
        _max_candidates: usize,
    ) -> Result<BackendQueryResult, String> {
        Err(self.not_ready_message())
    }

    fn commit_selection(
        &mut self,
        _committed_text: &str,
        _candidate_index: usize,
    ) -> Result<BackendCommitResult, String> {
        Err(self.not_ready_message())
    }

    fn reset(&mut self) -> Result<u64, String> {
        Ok(self.reset_for_new_session())
    }

    fn set_debug_timeline_enabled(&mut self, _enabled: bool) {}

    fn drain_debug_timeline(&mut self) -> Vec<String> {
        Vec::new()
    }
}
