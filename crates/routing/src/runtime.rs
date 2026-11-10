//! The process-wide tokio runtime and upstream HTTP client (agent-settings 04 §5.1).
//!
//! Created on first use; installs the rustls `ring` provider before any TLS client exists.

use std::{sync::LazyLock, time::Duration};

use thinkrail_core::{Error, ErrorKind, Result};

#[derive(Debug)]
pub(crate) struct Runtime {
    pub tokio: tokio::runtime::Runtime,
    /// Shared upstream client: system proxy respected, 10 s connect timeout, no total timeout
    /// (streamed responses can be long).
    pub client: reqwest::Client,
}

static RUNTIME: LazyLock<std::result::Result<Runtime, String>> = LazyLock::new(create);

fn create() -> std::result::Result<Runtime, String> {
    // Already installed (by us or the embedding process) is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("thinkrail-routing")
        .enable_all()
        .build()
        .map_err(|_| "无法创建网络运行时".to_owned())?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "无法创建 HTTP 客户端".to_owned())?;
    Ok(Runtime { tokio, client })
}

pub(crate) fn get() -> Result<&'static Runtime> {
    RUNTIME
        .as_ref()
        .map_err(|message| Error::new(ErrorKind::BackendUnavailable, message))
}
