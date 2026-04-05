use crate::backend::RimeBackend;
use crate::proto::rime_service_v2::{KeyEvent, RimeContextProto};
use std::sync::mpsc;
use std::thread;

enum BackendCommand {
    OpenSession {
        reply: mpsc::Sender<Option<usize>>,
    },
    DestroySession {
        session_id: usize,
    },
    ProcessKey {
        session_id: usize,
        key: KeyEvent,
        reply: mpsc::Sender<bool>,
    },
    GetContext {
        session_id: usize,
        reply: mpsc::Sender<RimeContextProto>,
    },
    GetCommit {
        session_id: usize,
        reply: mpsc::Sender<Option<String>>,
    },
}

pub struct ChannelRimeBackend {
    tx: mpsc::Sender<BackendCommand>,
}

impl ChannelRimeBackend {
    pub fn new(mut inner: Option<Box<dyn RimeBackend>>) -> Self {
        let (tx, rx) = mpsc::channel();
        
        // Take the inner backend if passed, or default (used in Windows IMM case)
        let _ = thread::spawn(move || {
            // Note: If inner is None, it assumes we create it ON this thread.
            // This is crucial for Win32 thread affinity. The window and IMM context
            // MUST be created on the exact thread that calls them.
            #[cfg(windows)]
            let mut runtime_backend = inner.take().unwrap_or_else(|| Box::new(crate::win_imm::ImmRimeAdapter::new()));
            
            #[cfg(not(windows))]
            let mut runtime_backend = inner.take().expect("Non-Windows requires a backend instance directly passed");

            for cmd in rx {
                match cmd {
                    BackendCommand::OpenSession { reply } => {
                        let _ = reply.send(runtime_backend.open_session());
                    }
                    BackendCommand::DestroySession { session_id } => {
                        runtime_backend.destroy_session(session_id);
                    }
                    BackendCommand::ProcessKey { session_id, key, reply } => {
                        let _ = reply.send(runtime_backend.process_key(session_id, &key));
                    }
                    BackendCommand::GetContext { session_id, reply } => {
                        let _ = reply.send(runtime_backend.get_context(session_id));
                    }
                    BackendCommand::GetCommit { session_id, reply } => {
                        let _ = reply.send(runtime_backend.get_commit(session_id));
                    }
                }
            }
        });

        Self { tx }
    }
}

impl RimeBackend for ChannelRimeBackend {
    fn open_session(&mut self) -> Option<usize> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(BackendCommand::OpenSession { reply: tx }).unwrap();
        rx.recv().unwrap_or(None)
    }

    fn destroy_session(&mut self, session_id: usize) {
        let _ = self.tx.send(BackendCommand::DestroySession { session_id });
    }

    fn process_key(&mut self, session_id: usize, key: &KeyEvent) -> bool {
        let (tx, rx) = mpsc::channel();
        self.tx.send(BackendCommand::ProcessKey { session_id, key: key.clone(), reply: tx }).unwrap();
        rx.recv().unwrap_or(false)
    }

    fn get_context(&mut self, session_id: usize) -> RimeContextProto {
        let (tx, rx) = mpsc::channel();
        self.tx.send(BackendCommand::GetContext { session_id, reply: tx }).unwrap();
        rx.recv().unwrap_or_default()
    }

    fn get_commit(&mut self, session_id: usize) -> Option<String> {
        let (tx, rx) = mpsc::channel();
        self.tx.send(BackendCommand::GetCommit { session_id, reply: tx }).unwrap();
        rx.recv().unwrap_or(None)
    }
}