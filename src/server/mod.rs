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

    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;

    Ok(())
}
