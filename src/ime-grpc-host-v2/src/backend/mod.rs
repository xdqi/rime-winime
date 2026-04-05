#[cfg(not(windows))]
pub mod rime_ffi;

#[cfg(not(windows))]
pub mod native;

use crate::proto::rime_service_v2::{KeyEvent, RimeContextProto};

pub trait RimeBackend: Send + Sync {
    /// Open a new Rime session and return the ID if successful.
    fn open_session(&mut self) -> Option<usize>;

    /// Destroy a Rime session by ID.
    fn destroy_session(&mut self, session_id: usize);

    /// Process a key event, returning true if consumed (accepted).
    fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool;

    /// Get the current context (composition, menu, pending commit text).
    fn get_context(&mut self, session_id: usize) -> RimeContextProto;

    /// Commit current selection/composition and retrieve the finalized text.
    fn get_commit(&mut self, session_id: usize) -> Option<String>;
}
