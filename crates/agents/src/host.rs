//! Engine glue for the host tools (04 §6): the per-launch environment, the ACP `mcpServers` entry,
//! OMP tool definitions, and the atomic writer for generated files.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};
use thinkrail_core::{Error, ErrorKind, HostToolAccess, IoContext, Result};

use crate::process::plain_path;

pub(crate) const SERVER: &str = "thinkrail";
pub(crate) const PIPE_VAR: &str = "THINKRAIL_HOST_PIPE";
pub(crate) const TOKEN_VAR: &str = "THINKRAIL_HOST_TOKEN";

/// Variables every engine launch gets when host tools are on.
pub(crate) fn env(access: &HostToolAccess) -> Vec<(String, OsString)> {
    vec![
        (
            "THINKRAIL_HOST_EXE".to_owned(),
            plain_path(&access.executable).into_os_string(),
        ),
        (PIPE_VAR.to_owned(), access.pipe.clone().into()),
        (TOKEN_VAR.to_owned(), access.token.clone().into()),
    ]
}

/// Writes `name` with `content` under `dir` (a launch's `StartSession.support_dir`, or OMP's
/// config root for `mcp.json`) unless it already holds exactly that: temporary file + rename.
pub(crate) fn write_support(dir: &Path, name: &str, content: &str) -> Result<PathBuf> {
    fs::create_dir_all(dir).context("无法创建引擎辅助目录")?;
    let path = dir.join(name);
    if fs::read_to_string(&path).ok().as_deref() != Some(content) {
        let temp = dir.join(format!("{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        fs::write(&temp, content).context("无法写入引擎辅助文件")?;
        fs::rename(&temp, &path).context("无法写入引擎辅助文件")?;
    }
    Ok(plain_path(&path))
}

/// The ACP `mcpServers` entry. DSH scrubs its parent's environment, so the pipe and token travel
/// explicitly in `env`.
pub(crate) fn acp_server(access: &HostToolAccess) -> Result<Value> {
    let command = plain_path(&access.executable);
    if !command.is_absolute() {
        return Err(Error::new(
            ErrorKind::BackendUnavailable,
            "宿主可执行文件路径不是绝对路径",
        ));
    }
    Ok(json!({
        "name": SERVER,
        "command": command,
        "args": ["mcp-host"],
        "env": [{"name": PIPE_VAR, "value": access.pipe}, {"name": TOKEN_VAR, "value": access.token}],
    }))
}

/// OMP `set_host_tools` definitions: every tool `essential`, or OMP keeps non-builtin tools
/// discoverable-only and the model never sees them.
pub(crate) fn omp_definitions(access: &HostToolAccess) -> Value {
    Value::Array(
        access
            .handler
            .specs()
            .into_iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "label": spec.name,
                    "description": spec.description,
                    "parameters": spec.input_schema,
                    "loadMode": "essential",
                })
            })
            .collect(),
    )
}
