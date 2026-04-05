use crate::backend::RimeBackend;
use crate::proto::rime_service_v2::{KeyEvent, RimeContextProto};
use std::sync::mpsc as std_mpsc;
use tokio::sync::oneshot;
use std::thread;

enum BackendCommand {
    OpenSession {
        reply: oneshot::Sender<Option<usize>>,
    },
    DestroySession {
        session_id: usize,
    },
    ProcessKey {
        session_id: usize,
        key: KeyEvent,
        reply: oneshot::Sender<bool>,
    },
    GetContext {
        session_id: usize,
        reply: oneshot::Sender<RimeContextProto>,
    },
    GetCommit {
        session_id: usize,
        reply: oneshot::Sender<Option<String>>,
    },
    SelectCandidate {
        session_id: usize,
        index: usize,
        reply: oneshot::Sender<bool>,
    },
}

pub struct ChannelRimeBackend {
    tx: std_mpsc::Sender<BackendCommand>,
}

impl ChannelRimeBackend {
    pub fn new(mut inner: Option<Box<dyn RimeBackend>>) -> Self {
        let (tx, rx) = std_mpsc::channel();
        
        let _ = thread::spawn(move || {
            let mut runtime_backend = inner.take().unwrap_or_else(|| Box::new(crate::win_imm::ImmRimeAdapter::default()));
            
            for cmd in rx {
                match cmd {
                    BackendCommand::OpenSession { reply } => {
                        // Ignoring if the receiver dropped
                        let _ = reply.send(tokio::runtime::Runtime::new().unwrap().block_on(runtime_backend.open_session()));
                    }
                    BackendCommand::DestroySession { session_id } => {
                        tokio::runtime::Runtime::new().unwrap().block_on(runtime_backend.destroy_session(session_id));
                    }
                    BackendCommand::ProcessKey { session_id, key, reply } => {
                        let ans = tokio::runtime::Runtime::new().unwrap().block_on(runtime_backend.process_key(session_id, &key));
                        let _ = reply.send(ans);
                    }
                    BackendCommand::GetContext { session_id, reply } => {
                        let _ = reply.send(tokio::runtime::Runtime::new().unwrap().block_on(runtime_backend.get_context(session_id)));
                    }
                    BackendCommand::GetCommit { session_id, reply } => {
                        let _ = reply.send(tokio::runtime::Runtime::new().unwrap().block_on(runtime_backend.get_commit(session_id)));
                    }
                    BackendCommand::SelectCandidate { session_id, index, reply } => {
                        let _ = reply.send(tokio::runtime::Runtime::new().unwrap().block_on(runtime_backend.select_candidate(session_id, index)));
                    }
                }
            }
        });

        Self {
            tx,
        }
    }
}

#[tonic::async_trait]
impl RimeBackend for ChannelRimeBackend {
    async fn open_session(&mut self) -> Option<usize> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(BackendCommand::OpenSession { reply: tx }).unwrap();
        rx.await.unwrap_or(None)
    }

    async fn destroy_session(&mut self, session_id: usize) {
        let _ = self.tx.send(BackendCommand::DestroySession { session_id });
    }

    async fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool {
        let (tx, rx) = oneshot::channel();
        self.tx.send(BackendCommand::ProcessKey { session_id, key: *key, reply: tx }).unwrap();
        rx.await.unwrap_or(false)
    }

    async fn get_context(&mut self, session_id: usize) -> RimeContextProto {
        let (tx, rx) = oneshot::channel();
        self.tx.send(BackendCommand::GetContext { session_id, reply: tx }).unwrap();
        rx.await.unwrap_or_default()
    }

    async fn get_commit(&mut self, session_id: usize) -> Option<String> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(BackendCommand::GetCommit { session_id, reply: tx }).unwrap();
        rx.await.unwrap_or(None)
    }

    async fn select_candidate(&mut self, session_id: usize, index: usize) -> bool {
        let (tx, rx) = oneshot::channel();
        self.tx.send(BackendCommand::SelectCandidate { session_id, index, reply: tx }).unwrap();
        rx.await.unwrap_or(false)
    }
}
