//! 编码搭档: the engine families, where each is installed, the default Agent's readiness and which
//! provider each engine uses (agent-settings 04 §7.10, 07).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{ProviderId, Result};

/// The external engine family a chat runs. Each has its own wire protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    /// Official pi CLI, `--mode rpc` JSONL.
    Pi,
    /// Official OMP CLI, `--mode rpc-ui` JSONL.
    Omp,
    /// DSH ACP JSON-RPC over stdio.
    Dsh,
}

impl BackendKind {
    pub const ALL: [Self; 3] = [Self::Pi, Self::Omp, Self::Dsh];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Omp => "OMP",
            Self::Dsh => "DSH",
        }
    }
}

/// How an installed engine process is started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Launch {
    /// An absolute executable (e.g. `node.exe`) plus argv (e.g. the official CLI entry).
    External {
        executable: PathBuf,
        args: Vec<String>,
    },
}

/// Where a found engine lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineSource {
    /// Installed by the app under its data directory, with the app's own runtime.
    Managed,
    /// Found on the user's `PATH`.
    System,
}

/// An installed engine and how to launch it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineInstall {
    pub source: EngineSource,
    pub launch: Launch,
    /// Known for managed installs (the pinned version); `None` for system ones.
    pub version: Option<String>,
}

pub trait EngineSetup: Send + Sync {
    /// The managed install if present, else the first `PATH` match, else `None`. No network.
    fn status(&self, backend: BackendKind) -> Result<Option<EngineInstall>>;
    /// Downloads the pinned runtime (checked against its SHA-256) and installs the pinned
    /// engine package under the data directory. Blocking; minutes on a first run.
    fn install(&self, backend: BackendKind) -> Result<EngineInstall>;
}

/// Whether new chats can start (07 §5.1): the default Agent, given what 编码搭档 finds installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready(BackendKind),
    /// No engine is installed.
    NoneInstalled,
    /// Some engine is installed (the first one, in [`BackendKind::ALL`] order), none is default.
    NoDefault(BackendKind),
    /// The default engine is no longer found.
    DefaultMissing(BackendKind),
}

/// `installed` lists the engines [`EngineSetup::status`] found.
pub fn readiness(default: Option<BackendKind>, installed: &[BackendKind]) -> Readiness {
    match default {
        Some(backend) if installed.contains(&backend) => Readiness::Ready(backend),
        Some(backend) => Readiness::DefaultMissing(backend),
        None => match BackendKind::ALL.into_iter().find(|b| installed.contains(b)) {
            Some(first) => Readiness::NoDefault(first),
            None => Readiness::NoneInstalled,
        },
    }
}

/// The single provider row chosen for an engine on the 编码搭档 page. With no choice an engine
/// gets every enabled provider, each by its own 接入方式 (the behaviour before this page).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "provider")]
pub enum EngineProvider {
    /// 统一网关: every enabled provider, all through the loopback gateway.
    Gateway,
    /// {engine} 官方: no provider is projected; the engine uses its own configuration.
    Official,
    /// One provider, with its own access mode.
    Provider(ProviderId),
}
