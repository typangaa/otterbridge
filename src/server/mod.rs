//! MCP server wiring.
//!
//! This module exposes [`WeirServer`] and provides the stdio transport entry
//! point [`run_stdio`], which drives the MCP protocol loop until the client
//! disconnects or the process exits.

pub mod tools;

pub use tools::WeirServer;

/// Run a [`WeirServer`] over stdin/stdout (the MCP stdio transport).
///
/// This function blocks until the MCP session ends. It should be called from
/// the `serve` subcommand after constructing a [`WeirServer`].
///
/// # Errors
/// Returns an error if the transport layer encounters an unrecoverable I/O
/// failure during the session.
pub async fn run_stdio(server: WeirServer) -> anyhow::Result<()> {
    use rmcp::ServiceExt as _;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::observability::MetricsPersister;

    // Grab the metrics handle before `server` is moved into `serve`.
    let metrics = server.metrics_handle();
    let persister = Arc::new(MetricsPersister::at_default_path());

    // Periodic flush of live metrics to disk (server-authoritative circuit state).
    let flush_metrics = metrics.clone();
    let flush_persister = persister.clone();
    let flush_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if let Err(e) = flush_persister.flush(&flush_metrics, true).await {
                tracing::debug!(error = %e, "periodic metrics flush failed");
            }
        }
    });

    let session = server.serve(rmcp::transport::stdio()).await?;
    let outcome = session.waiting().await;

    // Stop the periodic task and do one final flush on shutdown.
    flush_task.abort();
    if let Err(e) = persister.flush(&metrics, true).await {
        tracing::debug!(error = %e, "final metrics flush failed");
    }

    outcome?;
    Ok(())
}
