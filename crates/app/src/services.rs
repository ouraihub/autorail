//! Composition root: the only place concrete services are built. Everything else holds the
//! `core` traits.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context as _;
use thinkrail_agents::{DshRuntime, OmpRuntime, PiRuntime};
use serde_json::Value;
use thinkrail_core::{
    AgentRuntime, BackendKind, ChatId, HostToolAccess, HostToolHandler, HostToolResult, HostToolSpec,
    PlanReviewService, PlanScope, ProfileService, SessionEvent, SessionSink, TerminalEvent, TerminalService,
    TerminalSink, WorkspaceId, WorkspaceRoots, WorkspaceService,
};
use thinkrail_host_tools::{HostTools, ToolBridge};
use thinkrail_plan_review::JsonPlanReviewStore;
use thinkrail_profiles::JsonProfileStore;
use thinkrail_terminal::PtyTerminalService;
use thinkrail_workspace::GitWorkspaceService;
use tracing::{debug, warn};

/// PTY output waits here when the UI falls behind (backpressure on the reader thread).
const TERMINAL_QUEUE: usize = 256;

/// The host tool bridge plus what every chat's engine needs to reach it.
struct HostToolsHub {
    bridge: ToolBridge,
    handler: Arc<dyn HostToolHandler>,
    executable: PathBuf,
    support_dir: PathBuf,
}

#[derive(Clone)]
pub struct Services {
    pub workspace: Arc<dyn WorkspaceService>,
    pub profiles: Arc<dyn ProfileService>,
    pub terminals: Arc<dyn TerminalService>,
    pub plans: Arc<dyn PlanReviewService>,
    runtimes: [Arc<dyn AgentRuntime>; 3],
    /// `None` when the bridge could not start: chats then run without host tools.
    host_tools: Option<Arc<HostToolsHub>>,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services").finish_non_exhaustive()
    }
}

impl Services {
    pub fn runtime(&self, backend: BackendKind) -> Arc<dyn AgentRuntime> {
        match backend {
            BackendKind::Pi => self.runtimes[0].clone(),
            BackendKind::Omp => self.runtimes[1].clone(),
            BackendKind::Dsh => self.runtimes[2].clone(),
        }
    }

    /// A fresh token for one engine launch of `chat`; its tool calls land in this chat's scope.
    pub fn host_tools(&self, workspace: &WorkspaceId, chat: &ChatId) -> Option<HostToolAccess> {
        let hub = self.host_tools.as_ref()?;
        let token = hub.bridge.register(PlanScope { workspace: workspace.clone(), chat: chat.clone() });
        Some(HostToolAccess {
            executable: hub.executable.clone(),
            pipe: hub.bridge.pipe_name().to_owned(),
            token,
            support_dir: hub.support_dir.clone(),
            handler: hub.handler.clone(),
        })
    }
}

/// Engine and PTY threads push events here; foreground tasks drain them.
pub struct Channels {
    pub sessions: async_channel::Receiver<SessionEvent>,
    pub session_sink: Arc<dyn SessionSink>,
    pub terminals: async_channel::Receiver<TerminalEvent>,
    /// The chat whose plan a host tool call may have changed.
    pub plans: async_channel::Receiver<PlanScope>,
}

/// Tells the UI which chat's plan to re-read after every host tool call.
struct NotifyingTools {
    inner: HostTools,
    changed: async_channel::Sender<PlanScope>,
}

impl HostToolHandler for NotifyingTools {
    fn specs(&self) -> Vec<HostToolSpec> {
        self.inner.specs()
    }

    fn call(&self, scope: &PlanScope, name: &str, arguments: Value) -> thinkrail_core::Result<HostToolResult> {
        let result = self.inner.call(scope, name, arguments);
        if self.changed.try_send(scope.clone()).is_err() {
            debug!("plan change dropped: UI is gone");
        }
        result
    }
}

struct SessionChannel(async_channel::Sender<SessionEvent>);

impl SessionSink for SessionChannel {
    fn publish(&self, event: SessionEvent) {
        if self.0.try_send(event).is_err() {
            debug!("session event dropped: UI is gone");
        }
    }
}

struct TerminalChannel(async_channel::Sender<TerminalEvent>);

impl TerminalSink for TerminalChannel {
    fn publish(&self, event: TerminalEvent) {
        if self.0.send_blocking(event).is_err() {
            debug!("terminal event dropped: UI is gone");
        }
    }
}

pub fn build(data_dir: &Path) -> anyhow::Result<(Services, Channels)> {
    std::fs::create_dir_all(data_dir).with_context(|| format!("无法创建数据目录 {}", data_dir.display()))?;
    let workspace = Arc::new(GitWorkspaceService::open(data_dir).context("无法打开工作区数据")?);
    let roots: Arc<dyn WorkspaceRoots> = workspace.clone();
    let profiles = Arc::new(JsonProfileStore::open(data_dir).context("无法打开 Agent 档案")?);
    let plans: Arc<dyn PlanReviewService> =
        Arc::new(JsonPlanReviewStore::open(data_dir, roots.clone()).context("无法打开计划与审查数据")?);
    let (session_tx, sessions) = async_channel::unbounded();
    let (terminal_tx, terminals) = async_channel::bounded(TERMINAL_QUEUE);
    let (plan_tx, plan_changes) = async_channel::unbounded();
    let terminals_service = Arc::new(PtyTerminalService::new(data_dir, roots, Arc::new(TerminalChannel(terminal_tx))));
    let host_tools = start_host_tools(data_dir, plans.clone(), plan_tx);
    let services = Services {
        workspace,
        profiles,
        terminals: terminals_service,
        plans,
        runtimes: [Arc::new(PiRuntime::new()), Arc::new(OmpRuntime::new()), Arc::new(DshRuntime::new())],
        host_tools,
    };
    let channels =
        Channels { sessions, session_sink: Arc::new(SessionChannel(session_tx)), terminals, plans: plan_changes };
    Ok((services, channels))
}

/// Host tools are an enhancement: if the pipe cannot start, chats still run, without them.
fn start_host_tools(
    data_dir: &Path,
    plans: Arc<dyn PlanReviewService>,
    changed: async_channel::Sender<PlanScope>,
) -> Option<Arc<HostToolsHub>> {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            warn!(%error, "host tools disabled: own executable path unknown");
            return None;
        }
    };
    let handler: Arc<dyn HostToolHandler> = Arc::new(NotifyingTools { inner: HostTools::new(plans), changed });
    match ToolBridge::start(handler.clone()) {
        Ok(bridge) => Some(Arc::new(HostToolsHub { bridge, handler, executable, support_dir: data_dir.join("engines") })),
        Err(error) => {
            warn!(%error, "host tools disabled: bridge did not start");
            None
        }
    }
}
