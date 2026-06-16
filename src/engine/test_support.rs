//! Shared test doubles for the engine unit tests.
//!
//! Each engine resolves `Arc<dyn Backend>` handles and orchestrates them; these
//! mocks let a test script exactly what each backend returns and inspect every
//! request an engine sent (prompt content, role, call order). Only compiled
//! under `cfg(test)`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::backends::{Backend, ChatRequest, ChatResponse};
use crate::error::{Result, WeirError};

/// A scriptable mock backend.
///
/// `reply` is invoked as `reply(call_index, &request)` and returns either the
/// response content or an error; `call_index` is 0-based per backend instance,
/// which is what `eval_loop` tests use to drive a FAIL→PASS sequence. Every
/// request is captured in `received` before `reply` runs so a test can assert on
/// what the engine forwarded even when the call fails.
pub struct MockBackend {
    name: String,
    #[allow(clippy::type_complexity)]
    reply: Box<dyn Fn(usize, &ChatRequest) -> Result<String> + Send + Sync>,
    received: Arc<Mutex<Vec<ChatRequest>>>,
}

impl MockBackend {
    /// Build a mock with a custom per-call reply function.
    pub fn new(
        name: impl Into<String>,
        reply: impl Fn(usize, &ChatRequest) -> Result<String> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            reply: Box::new(reply),
            received: Arc::new(Mutex::new(Vec::new())),
        })
    }

    /// A backend that always replies with the same fixed content.
    pub fn echo(name: impl Into<String>, reply: impl Into<String>) -> Arc<Self> {
        let reply = reply.into();
        Self::new(name, move |_, _| Ok(reply.clone()))
    }

    /// A backend whose every call fails with [`WeirError::Backend`].
    pub fn failing(name: impl Into<String>) -> Arc<Self> {
        Self::new(name, |_, _| Err(WeirError::Backend("mock failure".into())))
    }

    /// Every request this backend received, in call order.
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.received.lock().unwrap().clone()
    }

    /// The user-message content of each received request, in call order.
    pub fn prompts(&self) -> Vec<String> {
        self.received
            .lock()
            .unwrap()
            .iter()
            .map(|r| {
                r.messages
                    .iter()
                    .find(|m| m.role == "user")
                    .map(|m| m.content.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Number of times this backend was called.
    pub fn call_count(&self) -> usize {
        self.received.lock().unwrap().len()
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse> {
        let idx = {
            let mut guard = self.received.lock().unwrap();
            guard.push(req.clone());
            guard.len() - 1
        };
        let content = (self.reply)(idx, &req)?;
        Ok(ChatResponse {
            content,
            backend_name: self.name.clone(),
            model: None,
            usage: None,
        })
    }

    async fn health(&self) -> Result<()> {
        Ok(())
    }
}
