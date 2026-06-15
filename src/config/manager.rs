//! Hot-reload config manager.
//!
//! [`ConfigManager`] wraps a [`Config`] in an [`ArcSwap`] so that any number of
//! readers can grab a consistent snapshot via [`ConfigManager::current`] with no
//! locking, while a background watcher quietly replaces the inner [`Arc`] whenever
//! the file changes on disk.
//!
//! # Guarantees
//! - Reads are always wait-free (ArcSwap::load is a single atomic load).
//! - A bad file or validation failure keeps the previous config in place — the
//!   gateway never goes down due to a broken hot-reload.
//! - The file watcher runs in a dedicated `tokio::spawn` task; no threads are
//!   created by this module.

use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::validate;
use crate::config::Config;
use crate::error::{Result, WeirError};

/// Shared, live-updated configuration.
///
/// Clone is intentionally cheap — the inner [`ArcSwap`] is reference-counted.
#[derive(Clone, Debug)]
pub struct ConfigManager {
    config: Arc<ArcSwap<Config>>,
    path: PathBuf,
}

impl ConfigManager {
    /// Create a new [`ConfigManager`] by loading and validating `path`.
    ///
    /// Returns an error if the file cannot be read, parsed, or fails validation.
    pub fn new(path: PathBuf) -> Result<Self> {
        let cfg = Config::load(&path)?;
        validate::validate(&cfg)?;
        Ok(Self {
            config: Arc::new(ArcSwap::from_pointee(cfg)),
            path,
        })
    }

    /// Return a snapshot of the current config.
    ///
    /// The returned [`Arc`] is stable — it will not change even if the file is
    /// reloaded between calls. Callers that need a consistent view across
    /// multiple operations should store the returned `Arc` for the duration of
    /// those operations.
    #[inline]
    pub fn current(&self) -> Arc<Config> {
        self.config.load_full()
    }

    /// Spawn a background tokio task that watches the config file for changes
    /// and hot-reloads it.
    ///
    /// The watcher is non-blocking: if a reload fails (I/O error, parse error,
    /// validation error) the current config is preserved and a warning is
    /// logged.  The task runs until the process exits.
    pub fn spawn_watcher(&self) -> Result<()> {
        let manager = self.clone();

        // Channel used to bridge the synchronous notify callback into async.
        // A buffer of 8 is plenty — we coalesce bursts anyway.
        let (tx, mut rx) = mpsc::channel::<()>(8);

        // Build the watcher on the *current thread* (synchronous) so we can
        // return an error immediately if inotify/kqueue initialisation fails.
        let path = manager.path.clone();
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                match res {
                    Ok(event) => {
                        // Only care about modifications.
                        if matches!(
                            event.kind,
                            EventKind::Modify(_)
                        ) {
                            // Best-effort send; if the channel is full we just
                            // drop the notification — the next modify will fire
                            // another event anyway.
                            let _ = tx.try_send(());
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "notify watcher error");
                    }
                }
            },
            notify::Config::default(),
        )
        .map_err(|e| WeirError::Config(format!("failed to create file watcher: {e}")))?;

        watcher
            .watch(&path, RecursiveMode::NonRecursive)
            .map_err(|e| {
                WeirError::Config(format!(
                    "failed to watch {}: {e}",
                    path.display()
                ))
            })?;

        tokio::spawn(async move {
            // Keep `watcher` alive inside the task — dropping it would
            // unregister the OS watch.
            let _watcher = watcher;

            while let Some(()) = rx.recv().await {
                // Coalesce rapid-fire events (e.g. editors that do a
                // write-then-rename) by draining the channel before reloading.
                while rx.try_recv().is_ok() {}

                match manager.reload_inner() {
                    Ok(()) => {
                        info!(path = %manager.path.display(), "config hot-reloaded successfully");
                    }
                    Err(e) => {
                        warn!(
                            path = %manager.path.display(),
                            error = %e,
                            "config reload failed — keeping previous config"
                        );
                    }
                }
            }

            // The channel closed (sender dropped), meaning the watcher was
            // torn down.  Nothing to do.
        });

        Ok(())
    }

    /// Manually reload the config from disk.
    ///
    /// On success the new config is swapped in atomically.
    /// On failure the existing config is preserved and the error is returned.
    #[allow(dead_code)] // public manual-reload API; watcher path is the default
    pub fn reload(&self) -> Result<()> {
        self.reload_inner()
    }

    // ------------------------------------------------------------------
    // Private helpers
    // ------------------------------------------------------------------

    /// Core reload logic shared by both the watcher and the manual path.
    fn reload_inner(&self) -> Result<()> {
        let new_cfg = Config::load(&self.path)?;
        validate::validate(&new_cfg)?;
        self.config.store(Arc::new(new_cfg));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write as _};
    use tempfile::NamedTempFile;

    /// Minimal valid TOML that passes all 3 validation layers.
    const VALID_TOML: &str = r#"
[server]
name = "weir"
transport = "stdio"
port = 3000

[[backend]]
name = "local"
type = "stdio-cli"
command = "hermes"
args = []
timeout_secs = 60
"#;

    /// TOML with a duplicate backend name (fails Layer 1).
    const DUPLICATE_BACKEND_TOML: &str = r#"
[[backend]]
name = "dupe"
type = "stdio-cli"
command = "a"

[[backend]]
name = "dupe"
type = "stdio-cli"
command = "b"
"#;

    fn write_toml(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn new_loads_valid_config() {
        let f = write_toml(VALID_TOML);
        let mgr = ConfigManager::new(f.path().to_path_buf()).unwrap();
        let cfg = mgr.current();
        assert_eq!(cfg.backends.len(), 1);
        assert_eq!(cfg.backends[0].name, "local");
    }

    #[test]
    fn new_rejects_invalid_config() {
        let f = write_toml(DUPLICATE_BACKEND_TOML);
        let err = ConfigManager::new(f.path().to_path_buf()).unwrap_err();
        assert!(err.to_string().contains("duplicate backend name"));
    }

    #[test]
    fn reload_updates_config() {
        let mut f = write_toml(VALID_TOML);
        let mgr = ConfigManager::new(f.path().to_path_buf()).unwrap();
        assert_eq!(mgr.current().backends[0].name, "local");

        // Overwrite with a different valid config.
        let updated = VALID_TOML.replace("\"local\"", "\"updated\"");
        f.seek(SeekFrom::Start(0)).unwrap();
        f.as_file_mut().set_len(0).unwrap();
        f.write_all(updated.as_bytes()).unwrap();
        f.flush().unwrap();

        mgr.reload().unwrap();
        assert_eq!(mgr.current().backends[0].name, "updated");
    }

    #[test]
    fn reload_preserves_old_config_on_error() {
        let mut f = write_toml(VALID_TOML);
        let mgr = ConfigManager::new(f.path().to_path_buf()).unwrap();

        // Overwrite with broken TOML.
        f.seek(SeekFrom::Start(0)).unwrap();
        f.as_file_mut().set_len(0).unwrap();
        f.write_all(b"this is not valid toml [[[").unwrap();
        f.flush().unwrap();

        let err = mgr.reload();
        assert!(err.is_err());
        // Original config must still be accessible.
        assert_eq!(mgr.current().backends[0].name, "local");
    }

    #[test]
    fn current_is_wait_free_and_cloneable() {
        let f = write_toml(VALID_TOML);
        let mgr = ConfigManager::new(f.path().to_path_buf()).unwrap();
        // Multiple concurrent reads — each gets its own Arc.
        let a = mgr.current();
        let b = mgr.current();
        assert_eq!(a.server.name, b.server.name);
    }
}
