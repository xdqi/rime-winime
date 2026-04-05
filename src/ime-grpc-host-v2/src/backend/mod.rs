#[cfg(not(windows))]
pub mod rime_ffi;

#[cfg(not(windows))]
pub mod native;

use crate::proto::rime_service_v2::{KeyEvent, RimeContextProto};

#[tonic::async_trait]
pub trait RimeBackend: Send + Sync {
    /// Open a new Rime session and return the ID if successful.
    async fn open_session(&mut self) -> Option<usize>;

    /// Destroy a Rime session by ID.
    async fn destroy_session(&mut self, session_id: usize);

    /// Process a key event, returning true if consumed (accepted).
    async fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool;

    /// Get the current context (composition, menu, pending commit text).
    async fn get_context(&mut self, session_id: usize) -> RimeContextProto;

    /// Commit current selection/composition and retrieve the finalized text.
    async fn get_commit(&mut self, session_id: usize) -> Option<String>;
    
    /// Select a candidate (trigger commit or modification on the IME).
    async fn select_candidate(&mut self, session_id: usize, index: usize) -> bool;
}
