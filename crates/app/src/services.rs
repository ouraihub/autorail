//! Composition root: the only place concrete services are built. Everything else holds the
//! `core` traits.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context as _;
use thinkrail_agents::{DshRuntime, ManagedEngines, OmpRuntime, PiRuntime};
use serde_json::Value;
use thinkrail_core::{
    AgentRuntime, AppPrefService, BackendKind, ChatBindingService, ChatId, Error, ErrorCode as _, HostToolAccess,
    HostToolHandler, HostToolResult, HostToolSpec, EngineSetup, McpService, ModelPrefService, ModelRouter,
    PlanReviewService, PlanScope, ProviderProbe, ProviderService, Readiness, SessionEvent, SessionSink, TerminalEvent,
    TerminalService, TerminalSink, WorkspaceId, WorkspaceRoots, WorkspaceService,
};
use thinkrail_observability::LogHandle;
use thinkrail_host_tools::{HostTools, ToolBridge};
use thinkrail_plan_review::JsonPlanReviewStore;
use thinkrail_routing::{Gateway, Probe, Router};
use thinkrail_settings_store::SqliteSettingsStore;
use thinkrail_terminal::PtyTerminalService;
use thinkrail_workspace::GitWorkspaceService;
use tracing::{debug, info, warn};

/// PTY output waits here when the UI falls behind (backpressure on the reader thread).
const TERMINAL_QUEUE: usize = 256;

/// The host tool bridge plus what every chat's engine needs to reach it.
struct HostToolsHub {
    bridge: ToolBridge,
    handler: Arc<dyn HostToolHandler>,
    executable: PathBuf,
}

#[derive(Clone)]
pub struct Services {
    pub workspace: Arc<dyn WorkspaceService>,
    /// Which engine each chat runs and its native session (`thinkrail.db`).
    pub bindings: Arc<dyn ChatBindingService>,
    pub terminals: Arc<dyn TerminalService>,
    pub plans: Arc<dyn PlanReviewService>,
    pub providers: Arc<dyn ProviderService>,
    pub mcp: Arc<dyn McpService>,
    pub prefs: Arc<dyn ModelPrefService>,
    pub router: Arc<dyn ModelRouter>,
    pub probe: Arc<dyn ProviderProbe>,
    /// 编码搭档: managed / system engine installs.
    pub engines: Arc<dyn EngineSetup>,
    /// Host preferences (log page switch, log level).
    pub app_prefs: Arc<dyn AppPrefService>,
    /// The installed logger: level switching and the log page's read-back.
    pub log: Arc<LogHandle>,
    runtimes: [Arc<dyn AgentRuntime>; 3],
    /// `None` when the bridge could not start: chats then run without host tools.
    host_tools: Option<Arc<HostToolsHub>>,
    /// `<data>/engines`: generated engine glue, partitioned by chat.
    engine_support_dir: PathBuf,
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

    /// The default Agent against what 编码搭档 finds installed (agent-settings 07 §5.1). Blocking,
    /// no network; `Ready` also hands back the launch to use.
    pub fn readiness(&self) -> thinkrail_core::Result<(Readiness, Option<thinkrail_core::Launch>)> {
        let default = self.prefs.prefs()?.default_agent;
        let mut installed = Vec::new();
        let mut launch = None;
        for backend in BackendKind::ALL {
            if let Some(install) = self.engines.status(backend)? {
                installed.push(backend);
                if Some(backend) == default {
                    launch = Some(install.launch);
                }
            }
        }
        Ok((thinkrail_core::readiness(default, &installed), launch))
    }

    /// The install of `backend` for resuming a bound chat; `None` when 编码搭档 no longer finds it.
    pub fn launch(&self, backend: BackendKind) -> thinkrail_core::Result<Option<thinkrail_core::Launch>> {
        Ok(self.engines.status(backend)?.map(|install| install.launch))
    }

    /// A fresh token for one engine launch of `chat`; its tool calls land in this chat's scope.
    pub fn host_tools(&self, workspace: &WorkspaceId, chat: &ChatId) -> Option<HostToolAccess> {
        let hub = self.host_tools.as_ref()?;
        let token = hub.bridge.register(PlanScope { workspace: workspace.clone(), chat: chat.clone() });
        Some(HostToolAccess {
            executable: hub.executable.clone(),
            pipe: hub.bridge.pipe_name().to_owned(),
            token,
            handler: hub.handler.clone(),
        })
    }

    /// Creates this chat's isolated generated-engine directory.
    pub fn support_dir(&self, chat: &ChatId) -> thinkrail_core::Result<PathBuf> {
        let path = self.engine_support_dir.join(chat.as_str());
        std::fs::create_dir_all(&path).map_err(|source| Error::Io {
            context: format!("无法创建 Agent 辅助目录 {}", path.display()),
            source,
        })?;
        Ok(path)
    }

    /// Removes generated files while leaving native engine history untouched.
    pub fn remove_support_dir(&self, chat: &ChatId) -> thinkrail_core::Result<()> {
        let path = self.engine_support_dir.join(chat.as_str());
        match std::fs::remove_dir_all(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io {
                context: format!("无法删除 Agent 辅助目录 {}", path.display()),
                source,
            }),
        }
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

    /// Every engine's tool calls pass here (MCP bridge and OMP in-process): one line per call,
    /// tool name, chat, duration and outcome only — never arguments or results (04 §3).
    fn call(&self, scope: &PlanScope, name: &str, arguments: Value) -> thinkrail_core::Result<HostToolResult> {
        let started = std::time::Instant::now();
        let result = self.inner.call(scope, name, arguments);
        let ms = started.elapsed().as_millis();
        match &result {
            Ok(reply) => info!(chat = %scope.chat, tool = name, ms, is_error = reply.is_error, "host tool called"),
            Err(error) => warn!(chat = %scope.chat, tool = name, ms, code = error.code(), "host tool failed"),
        }
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

pub fn build(data_dir: &Path, db_path: &Path, log: Arc<LogHandle>) -> anyhow::Result<(Services, Channels)> {
    std::fs::create_dir_all(data_dir).with_context(|| format!("无法创建数据目录 {}", data_dir.display()))?;
    let workspace = Arc::new(GitWorkspaceService::open(data_dir).context("无法打开工作区数据")?);
    let roots: Arc<dyn WorkspaceRoots> = workspace.clone();
    let plans: Arc<dyn PlanReviewService> =
        Arc::new(JsonPlanReviewStore::open(data_dir, roots.clone()).context("无法打开计划与审查数据")?);
    let settings = Arc::new(SqliteSettingsStore::open(db_path).context("无法打开设置数据库；请检查数据库目录是否可写，或使用 --data-dir <目录> 指定可写目录")?);
    let providers: Arc<dyn ProviderService> = settings.clone();
    let mcp: Arc<dyn McpService> = settings.clone();
    let prefs: Arc<dyn ModelPrefService> = settings.clone();
    let bindings: Arc<dyn ChatBindingService> = settings.clone();
    let app_prefs: Arc<dyn AppPrefService> = settings.clone();
    // Logging started at `info` before the database existed; now apply the stored level.
    match app_prefs.app_prefs() {
        Ok(stored) => {
            if let Err(error) = log.set_level(stored.log_level) {
                warn!(error = %format!("{error:#}"), "stored log level not applied");
            }
        }
        Err(error) => warn!(code = error.code(), "app preferences not read; logging stays at info"),
    }
    let gateway = Gateway::new(providers.clone());
    let router: Arc<dyn ModelRouter> = Arc::new(Router::new(providers.clone(), mcp.clone(), prefs.clone(), gateway));
    let probe: Arc<dyn ProviderProbe> = Arc::new(Probe::new(providers.clone()));
    let (session_tx, sessions) = async_channel::unbounded();
    let (terminal_tx, terminals) = async_channel::bounded(TERMINAL_QUEUE);
    let (plan_tx, plan_changes) = async_channel::unbounded();
    let terminals_service = Arc::new(PtyTerminalService::new(data_dir, roots, Arc::new(TerminalChannel(terminal_tx))));
    let host_tools = start_host_tools(plans.clone(), workspace.clone(), plan_tx);
    let services = Services {
        workspace,
        bindings,
        terminals: terminals_service,
        plans,
        providers,
        mcp,
        prefs,
        router,
        probe,
        engines: Arc::new(ManagedEngines::new(data_dir)),
        app_prefs,
        log,
        runtimes: [Arc::new(PiRuntime::new()), Arc::new(OmpRuntime::new()), Arc::new(DshRuntime::new())],
        host_tools,
        engine_support_dir: data_dir.join("engines"),
    };
    let channels = Channels { sessions, session_sink: Arc::new(SessionChannel(session_tx)), terminals, plans: plan_changes };
    Ok((services, channels))
}

/// Host tools are an enhancement: if the pipe cannot start, chats still run, without them.
fn start_host_tools(
    plans: Arc<dyn PlanReviewService>,
    roots: Arc<dyn WorkspaceRoots>,
    changed: async_channel::Sender<PlanScope>,
) -> Option<Arc<HostToolsHub>> {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            warn!(%error, "host tools disabled: own executable path unknown");
            return None;
        }
    };
    let handler: Arc<dyn HostToolHandler> = Arc::new(NotifyingTools { inner: HostTools::new(plans, roots), changed });
    match ToolBridge::start(handler.clone()) {
        Ok(bridge) => Some(Arc::new(HostToolsHub { bridge, handler, executable })),
        Err(error) => {
            warn!(%error, "host tools disabled: bridge did not start");
            None
        }
    }
}
