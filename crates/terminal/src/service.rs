use std::{collections::HashMap, path::Path, sync::Arc};

use parking_lot::Mutex;
use thinkrail_core::{
    AttachRequest, Attached, Error, ErrorCode, GridSize, Result, TerminalRuntimeId, TerminalService, TerminalSink,
    TerminalTab, WorkspaceId, WorkspaceRoots,
};
use tracing::{info, warn};

use crate::{
    catalog::{Catalog, MAX_TITLE_BYTES, StoredTab, validate_ids},
    runtime::{Runtime, SpawnRequest},
};

/// [`TerminalService`] over portable-pty. Tabs persist in `<data>/terminals.json`; PTYs never do.
pub struct PtyTerminalService {
    catalog: Catalog,
    roots: Arc<dyn WorkspaceRoots>,
    sink: Arc<dyn TerminalSink>,
    runtimes: Mutex<HashMap<TerminalRuntimeId, Arc<Runtime>>>,
}

impl std::fmt::Debug for PtyTerminalService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyTerminalService").field("runtimes", &self.runtimes.lock().len()).finish_non_exhaustive()
    }
}

impl PtyTerminalService {
    pub fn new(data_dir: &Path, roots: Arc<dyn WorkspaceRoots>, sink: Arc<dyn TerminalSink>) -> Self {
        Self { catalog: Catalog::new(data_dir.to_path_buf()), roots, sink, runtimes: Mutex::new(HashMap::new()) }
    }

    /// Live runtime for a tab; a shell that exited on its own is dropped here, never returned.
    fn live(&self, workspace: &WorkspaceId, tab_key: &str) -> Option<Arc<Runtime>> {
        let mut runtimes = self.runtimes.lock();
        runtimes.retain(|_, runtime| !runtime.is_exited());
        runtimes.values().find(|r| &r.workspace == workspace && r.tab_key == tab_key).cloned()
    }

    fn runtime(&self, id: &TerminalRuntimeId) -> Result<Arc<Runtime>> {
        self.runtimes.lock().get(id).filter(|r| !r.is_exited()).cloned().ok_or_else(|| Error::not_found("终端"))
    }
}

impl TerminalService for PtyTerminalService {
    fn tabs(&self, workspace: &WorkspaceId) -> Result<Vec<TerminalTab>> {
        let tabs = self.catalog.lock()?.read()?;
        Ok(tabs
            .into_iter()
            .filter(|tab| &tab.workspace == workspace)
            .map(|tab| {
                let runtime = self.live(&tab.workspace, &tab.tab_key).map(|r| r.id.clone());
                TerminalTab { workspace: tab.workspace, tab_key: tab.tab_key, title: tab.title, runtime }
            })
            .collect())
    }

    fn reserve(&self, workspace: &WorkspaceId, tab_key: &str, title: &str) -> Result<TerminalTab> {
        validate_ids(workspace, tab_key)?;
        if title.len() > MAX_TITLE_BYTES {
            return Err(Error::invalid("终端标题过长"));
        }
        let locked = self.catalog.lock()?;
        let mut tabs = locked.read()?;
        let stored = match tabs.iter().find(|t| &t.workspace == workspace && t.tab_key == tab_key) {
            Some(existing) => existing.clone(),
            None => {
                let tab = StoredTab { workspace: workspace.clone(), tab_key: tab_key.into(), title: title.into() };
                tabs.push(tab.clone());
                locked.save(&tabs)?;
                tab
            }
        };
        let runtime = self.live(workspace, tab_key).map(|r| r.id.clone());
        Ok(TerminalTab { workspace: stored.workspace, tab_key: stored.tab_key, title: stored.title, runtime })
    }

    fn attach(&self, request: AttachRequest) -> Result<Attached> {
        validate_ids(&request.workspace, &request.tab_key)?;
        crate::runtime::validate_size(request.size)?;
        if let Some(runtime) = self.live(&request.workspace, &request.tab_key) {
            return Ok(runtime.attached(false));
        }
        let cwd = self.roots.workspace_root(&request.workspace)?;
        let locked = self.catalog.lock()?;
        let mut tabs = locked.read()?;
        // Re-check under the catalog lock so two attaches cannot start two shells for one tab.
        if let Some(runtime) = self.live(&request.workspace, &request.tab_key) {
            return Ok(runtime.attached(false));
        }
        let runtime = Arc::new(Runtime::spawn(SpawnRequest {
            workspace: request.workspace.clone(),
            tab_key: request.tab_key.clone(),
            shell: request.shell,
            size: request.size,
            cwd: &cwd,
            sink: self.sink.clone(),
        })?);
        if !tabs.iter().any(|t| t.workspace == request.workspace && t.tab_key == request.tab_key) {
            tabs.push(StoredTab { workspace: request.workspace, tab_key: request.tab_key, title: "终端".into() });
            locked.save(&tabs)?;
        }
        self.runtimes.lock().insert(runtime.id.clone(), runtime.clone());
        Ok(runtime.attached(true))
    }

    fn write(&self, runtime: &TerminalRuntimeId, data: &str) -> Result<()> {
        self.runtime(runtime)?.write(data)
    }

    fn resize(&self, runtime: &TerminalRuntimeId, size: GridSize) -> Result<()> {
        self.runtime(runtime)?.resize(size)
    }

    fn close(&self, workspace: &WorkspaceId, tab_key: &str) -> Result<()> {
        validate_ids(workspace, tab_key)?;
        let removed = {
            let mut runtimes = self.runtimes.lock();
            let id = runtimes.iter().find(|(_, r)| &r.workspace == workspace && r.tab_key == tab_key).map(|(id, _)| id.clone());
            id.and_then(|id| runtimes.remove(&id))
        };
        if let Some(runtime) = removed {
            runtime.shutdown()?;
        }
        let locked = self.catalog.lock()?;
        let mut tabs = locked.read()?;
        let before = tabs.len();
        tabs.retain(|t| !(&t.workspace == workspace && t.tab_key == tab_key));
        if tabs.len() != before {
            locked.save(&tabs)?;
        }
        info!(workspace = %workspace, "terminal tab closed");
        Ok(())
    }

    fn shutdown(&self) {
        let runtimes: Vec<_> = self.runtimes.lock().drain().map(|(_, r)| r).collect();
        for runtime in runtimes {
            if let Err(error) = runtime.shutdown() {
                warn!(runtime = %runtime.id, code = error.code(), "terminal shutdown failed");
            }
        }
    }
}

impl Drop for PtyTerminalService {
    fn drop(&mut self) {
        self.shutdown();
    }
}
