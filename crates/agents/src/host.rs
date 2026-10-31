//! Engine glue for the host tools (04 §6): the pi extension that registers the `mcp-host` server,
//! the DSH patch that turns off its built-in TODO tool, and the per-launch environment.

use std::{ffi::OsString, fs, path::PathBuf};

use serde_json::{Value, json};
use thinkrail_core::{Error, ErrorKind, HostToolAccess, IoContext, Result};

use crate::process::plain_path;

pub(crate) const SERVER: &str = "thinkrail";
pub(crate) const PIPE_VAR: &str = "THINKRAIL_HOST_PIPE";
pub(crate) const TOKEN_VAR: &str = "THINKRAIL_HOST_TOKEN";

/// `exposure: "direct"` declares the tools to the model (the default codemode hides them);
/// `timeout` 600 s leaves room for long tool runs. The child inherits pi's environment, which
/// carries `THINKRAIL_HOST_PIPE` / `THINKRAIL_HOST_TOKEN`.
const PI_EXTENSION: &str = r#"export default function (pi) {
	pi.registerMcpServer("thinkrail", {
		command: process.env.THINKRAIL_HOST_EXE,
		args: ["mcp-host"],
		exposure: "direct",
		timeout: 600,
		description: "ThinkRail host: this chat's TODO plan and review comments.",
	});
}
"#;

const DSH_PATCH: &str = "- id: tool-todo\n  disabled: true\n";

/// Variables every engine launch gets when host tools are on.
pub(crate) fn env(access: &HostToolAccess) -> Vec<(&'static str, OsString)> {
    vec![
        ("THINKRAIL_HOST_EXE", plain_path(&access.executable).into_os_string()),
        (PIPE_VAR, access.pipe.clone().into()),
        (TOKEN_VAR, access.token.clone().into()),
    ]
}

/// Writes `name` with `content` under the support directory unless it already holds exactly that.
fn support_file(access: &HostToolAccess, name: &str, content: &str) -> Result<PathBuf> {
    fs::create_dir_all(&access.support_dir).context("无法创建宿主工具目录")?;
    let path = access.support_dir.join(name);
    if fs::read_to_string(&path).ok().as_deref() != Some(content) {
        let temp = access.support_dir.join(format!("{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        fs::write(&temp, content).context("无法写入宿主工具文件")?;
        fs::rename(&temp, &path).context("无法写入宿主工具文件")?;
    }
    Ok(plain_path(&path))
}

pub(crate) fn pi_extension(access: &HostToolAccess) -> Result<PathBuf> {
    support_file(access, "pi-host-tools.js", PI_EXTENSION)
}

pub(crate) fn dsh_patch(access: &HostToolAccess) -> Result<PathBuf> {
    support_file(access, "dsh-host.patch.yml", DSH_PATCH)
}

/// The ACP `mcpServers` entry. DSH scrubs its parent's environment, so the pipe and token travel
/// explicitly in `env`.
pub(crate) fn acp_server(access: &HostToolAccess) -> Result<Value> {
    let command = plain_path(&access.executable);
    if !command.is_absolute() {
        return Err(Error::new(ErrorKind::BackendUnavailable, "宿主可执行文件路径不是绝对路径"));
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
