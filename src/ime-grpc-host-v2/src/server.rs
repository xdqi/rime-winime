use crate::backend::RimeBackend;
use crate::proto::rime_service_v2::{
    rime_service_server::RimeService, DestroySessionRequest, DestroySessionResponse,
    GetCommitRequest, GetCommitResponse, GetContextRequest, GetContextResponse, OpenSessionRequest,
    OpenSessionResponse, ProcessKeyRequest, ProcessKeyResponse,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::{Request, Response, Status};

pub struct RimeServerImpl {
    backend: Arc<Mutex<Box<dyn RimeBackend>>>,
}

impl RimeServerImpl {
    pub fn new(backend: Box<dyn RimeBackend>) -> Self {
        Self {
            backend: Arc::new(Mutex::new(backend)),
        }
    }
}

#[tonic::async_trait]
impl RimeService for RimeServerImpl {
    async fn open_session(
        &self,
        _request: Request<OpenSessionRequest>,
    ) -> Result<Response<OpenSessionResponse>, Status> {
        let mut backend = self.backend.lock().await;
        if let Some(id) = backend.open_session() {
            Ok(Response::new(OpenSessionResponse { session_id: id.to_string() }))
        } else {
            Err(Status::internal("Failed to open session"))
        }
    }

    async fn process_key(
        &self,
        request: Request<ProcessKeyRequest>,
    ) -> Result<Response<ProcessKeyResponse>, Status> {
        let req = request.into_inner();
        let session_id = req.session_id.parse::<usize>().unwrap_or(0);
        let mut backend = self.backend.lock().await;

        if let Some(key_event) = req.key_event {
            let accepted = backend.process_key(session_id, &key_event);
            Ok(Response::new(ProcessKeyResponse {
                session_id: req.session_id,
                accepted,
            }))
        } else {
            Err(Status::invalid_argument("Missing key_event"))
        }
    }

    async fn get_context(
        &self,
        request: Request<GetContextRequest>,
    ) -> Result<Response<GetContextResponse>, Status> {
        let req = request.into_inner();
        let session_id = req.session_id.parse::<usize>().unwrap_or(0);
        let mut backend = self.backend.lock().await;

        let context = backend.get_context(session_id);
        Ok(Response::new(GetContextResponse {
            session_id: req.session_id,
            context: Some(context),
        }))
    }

    async fn get_commit(
        &self,
        request: Request<GetCommitRequest>,
    ) -> Result<Response<GetCommitResponse>, Status> {
        let req = request.into_inner();
        let session_id = req.session_id.parse::<usize>().unwrap_or(0);
        let mut backend = self.backend.lock().await;

        let (commit_text, has_commit) = match backend.get_commit(session_id) {
            Some(text) => (text, true),
            None => (String::new(), false),
        };

        Ok(Response::new(GetCommitResponse {
            session_id: req.session_id,
            commit_text,
            has_commit,
        }))
    }

    async fn destroy_session(
        &self,
        request: Request<DestroySessionRequest>,
    ) -> Result<Response<DestroySessionResponse>, Status> {
        let req = request.into_inner();
        let session_id = req.session_id.parse::<usize>().unwrap_or(0);
        let mut backend = self.backend.lock().await;
        
        backend.destroy_session(session_id);
        
        Ok(Response::new(DestroySessionResponse { success: true }))
    }
}
