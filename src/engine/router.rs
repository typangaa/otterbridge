//! Router engine: direct a request to a single, explicitly-chosen backend.
//!
//! This is a thin wrapper that exists for API symmetry with the other engines
//! (`fan_out`, `pipeline`, `eval_loop`). The caller is responsible for
//! resolving the [`Backend`] handle before calling [`run`].

use std::sync::Arc;

use tracing::info;

use crate::backends::{Backend, ChatRequest, ChatResponse};
use crate::error::Result;

/// Forward `req` to `backend` and return its response.
///
/// Logs the backend name and the first 80 characters of the user prompt before
/// issuing the call so that structured logs capture the routing decision.
pub async fn run(backend: Arc<dyn Backend>, req: ChatRequest) -> Result<ChatResponse> {
    // Extract a preview of the first user message for the log line.
    let prompt_preview: String = req
        .messages
        .iter()
        .find(|m| m.role == "user")
        .map(|m| m.content.chars().take(80).collect())
        .unwrap_or_else(|| "<no user message>".to_string());

    info!(
        backend = %backend.name(),
        prompt_preview = %prompt_preview,
        "router: dispatching request"
    );

    let resp = backend.chat(req).await?;

    info!(
        backend = %resp.backend_name,
        model = ?resp.model,
        content_preview = %resp.content.chars().take(80).collect::<String>(),
        "router: received response"
    );

    Ok(resp)
}
