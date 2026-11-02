use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thinkrail_core::{
    Error, IoContext, Project, ProjectId, Result, Workspace, WorkspaceId, WorkspaceKind,
};

use crate::paths::{bounded_read, is_link};

const FILE_NAME: &str = "workbench.json";
const LOCK_NAME: &str = "workbench.json.lock";
const SCHEMA_VERSION: u32 = 1;
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// Proof that a managed worktree was created by this app; removal checks every field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Ownership {
    pub path: PathBuf,
    pub project_id: ProjectId,
    pub branch: String,
    pub common_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredProject {
    #[serde(flatten)]
    pub project: Project,
    pub closed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Registry {
    schema_version: u32,
    pub projects: Vec<StoredProject>,
    pub workspaces: Vec<Workspace>,
    pub managed: BTreeMap<WorkspaceId, Ownership>,
}

impl Registry {
    pub fn project(&self, id: &ProjectId) -> Result<&Project> {
        self.projects
            .iter()
            .find(|p| &p.project.id == id && !p.closed)
            .map(|p| &p.project)
            .ok_or_else(|| Error::not_found("项目"))
    }

    pub fn workspace(&self, id: &WorkspaceId) -> Result<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| &w.id == id)
            .ok_or_else(|| Error::not_found("工作区"))
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::corrupt(format!(
                "工作区注册表版本 {} 不受支持",
                self.schema_version
            )));
        }
        let mut ids = HashSet::new();
        for stored in &self.projects {
            let p = &stored.project;
            if !ids.insert(p.id.as_str())
                || !p.path.is_absolute()
                || p.slug.is_empty()
                || p.slug.contains(['/', '\\'])
            {
                return Err(Error::corrupt("工作区注册表中的项目记录无效"));
            }
        }
        for w in &self.workspaces {
            let owned_project = self.projects.iter().any(|p| p.project.id == w.project_id);
            if !ids.insert(w.id.as_str()) || !owned_project || !w.worktree_path.is_absolute() {
                return Err(Error::corrupt("工作区注册表中的工作区记录无效或重复"));
            }
            if (w.kind == WorkspaceKind::Managed) != self.managed.contains_key(&w.id) {
                return Err(Error::corrupt(
                    "托管工作区缺少归属记录，或归属记录没有对应工作区",
                ));
            }
        }
        Ok(())
    }
}

/// `workbench.json`. Reads see other processes' writes; writers hold an fs2 lock and re-read.
#[derive(Debug)]
pub(crate) struct Store {
    dir: PathBuf,
}

#[derive(Debug)]
pub(crate) struct Locked {
    _file: File,
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    pub fn read(&self) -> Result<Registry> {
        let path = self.dir.join(FILE_NAME);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !is_link(&metadata) => {}
            Ok(_) => return Err(Error::corrupt("workbench.json 不是普通文件")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Registry {
                    schema_version: SCHEMA_VERSION,
                    ..Registry::default()
                });
            }
            Err(error) => {
                return Err(Error::Io {
                    context: "读取工作区注册表".into(),
                    source: error,
                });
            }
        }
        let bytes = bounded_read(File::open(&path).context("读取工作区注册表")?, MAX_BYTES)
            .map_err(|_| Error::corrupt("工作区注册表超过 16 MiB"))?;
        let registry: Registry = serde_json::from_slice(&bytes)
            .map_err(|e| Error::corrupt(format!("工作区注册表无法解析：{e}")))?;
        registry.validate()?;
        Ok(registry)
    }

    pub fn lock(&self) -> Result<Locked> {
        fs::create_dir_all(&self.dir).context("创建数据目录")?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join(LOCK_NAME))
            .context("打开工作区注册表锁")?;
        fs2::FileExt::lock_exclusive(&file).context("锁定工作区注册表")?;
        Ok(Locked { _file: file })
    }

    pub fn write(&self, _lock: &Locked, registry: &Registry) -> Result<()> {
        registry.validate()?;
        let bytes = serde_json::to_vec_pretty(registry)
            .map_err(|e| Error::invalid(format!("无法序列化注册表：{e}")))?;
        let target = self.dir.join(FILE_NAME);
        let temporary = self
            .dir
            .join(format!("{FILE_NAME}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &target)
        })()
        .context("保存工作区注册表");
        if result.is_err()
            && let Err(error) = fs::remove_file(&temporary)
        {
            tracing::debug!(%error, "registry temporary cleanup failed");
        }
        result
    }
}
