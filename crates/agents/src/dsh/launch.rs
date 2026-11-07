//! DSH launch configuration: `--profile acp` arguments, the generated patch merged with the
//! user's provider routes, the config root and the MCP servers sent with the session.

use std::path::{Path, PathBuf};

use crate::{
    host,
    process::{self, LaunchSpec},
    projection_engine, workflow,
};
use serde_json::{Value, json};
use thinkrail_core::{Error, IoContext, Result, SessionRole, StartSession};

/// Host arguments and environment of one DSH launch: the generated patch (bundled skills,
/// workflow rules, built-in TODO tool off, providers, `acp` default model). Also returns the
/// staged bundled skills dir; the spec's `cwd` is the DSH home.
pub(super) fn launch_spec(request: &StartSession) -> Result<(LaunchSpec<'_>, PathBuf)> {
    let home = dsh_home(request)?;
    let reviewer = matches!(request.role, SessionRole::Reviewer { .. });
    let mut host_args = vec!["--profile".to_owned(), "acp".to_owned()];
    let host_env: Vec<_> = process::routing_env(&request.routing).collect();
    let (patch, staged) = if reviewer {
        let prompt = match &request.role {
            SessionRole::Reviewer { system_prompt } => system_prompt,
            SessionRole::Worker => unreachable!(),
        };
        (
            projection_engine::dsh_reviewer_patch(prompt, &request.routing)?,
            home.clone(),
        )
    } else {
        let skills = workflow::stage_skills(&request.support_dir)?;
        (
            projection_engine::dsh_patch(request.host_tools.is_some(), &request.routing, &skills)?,
            skills,
        )
    };
    let patch = preserve_user_routes(&home, patch, &mut host_env.clone())?;
    let path = host::write_support(
        &request.support_dir,
        projection_engine::DSH_PATCH_FILE,
        &patch,
    )?;
    host_args.extend(["--patch".to_owned(), path.display().to_string()]);
    let spec = LaunchSpec {
        launch: &request.launch,
        config_root: request.config_root.as_deref(),
        host_args,
        cwd: home,
        config_var: "DSH_HOME",
        host_env,
    };
    Ok((spec, staged))
}

pub(super) fn preserve_user_routes(
    home: &Path,
    patch: String,
    env: &mut Vec<(String, std::ffi::OsString)>,
) -> Result<String> {
    use serde_yaml_ng::Value as Yaml;
    if !patch.contains("- id: llm-pi-ai\n") {
        return Ok(patch);
    }
    let parse = |text: &str| {
        let text = if text.trim_start().starts_with("%") {
            text.to_owned()
        } else {
            format!(
                "%TAG !! !\n---\n{}",
                text.trim_start().strip_prefix("---").unwrap_or(text)
            )
        };
        serde_yaml_ng::from_str::<Yaml>(&text)
            .map_err(|_| Error::invalid("DSH 覆盖层不是有效的 YAML，请检查 cordis.patch.yml"))
    };
    let mut original = serde_json::Map::new();
    for path in [
        home.join("profiles/acp/cordis.patch.yml"),
        home.join("cordis.patch.yml"),
    ] {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(Error::invalid(
                    "无法读取 DSH 用户覆盖层，请检查配置目录权限",
                ));
            }
        };
        let parsed = parse(&text)?;
        if let Some(entries) = parsed.as_sequence() {
            for entry in entries {
                if entry["id"].as_str() == Some("llm-pi-ai")
                    && let Some(config) = entry.get("config")
                {
                    original = match config {
                        Yaml::Null => serde_json::Map::new(),
                        Yaml::Tagged(_) => {
                            let value = yaml_expressions(config)?;
                            serde_json::Map::from_iter([(
                                "__jsExpr".into(),
                                value["__jsExpr"].clone(),
                            )])
                        }
                        _ => yaml_expressions(config)?
                            .as_object()
                            .cloned()
                            .ok_or_else(|| Error::invalid("DSH 提供方 config 必须是对象"))?,
                    };
                }
            }
        }
    }
    if original.is_empty() {
        return Ok(patch);
    }
    let projected = parse(&patch)?;
    let mut output = Vec::new();
    for entry in projected
        .as_sequence()
        .ok_or_else(|| Error::protocol("DSH 投影不是列表"))?
    {
        if entry["id"].as_str() == Some("llm-pi-ai") {
            let projected = serde_json::to_value(&entry["config"])
                .map_err(|_| Error::protocol("无法编码 DSH 提供方投影"))?;
            let value = serde_json::to_string(&json!({"base": original, "projected": projected}))
                .map_err(|_| Error::protocol("无法编码 DSH 合并配置"))?;
            env.push(("THINKRAIL_DSH_MERGED_CONFIG".into(), value.into()));
            output.push(concat!("- id: llm-pi-ai\n  config: !!js >-\n    ",
                "((raw) => { const visit = (v) => v && typeof v === \"object\" ? ",
                "(typeof v.__jsExpr === \"string\" ? eval(v.__jsExpr) : ",
                "Array.isArray(v) ? v.map(visit) : Object.fromEntries(Object.entries(v).map(([k,x]) => [k,visit(x)]))) : v; ",
                "const base = visit(raw.base); return { ...base, providers: { ...base.providers, ...raw.projected.providers } }; ",
                "})(JSON.parse(process.env.THINKRAIL_DSH_MERGED_CONFIG))\n").to_owned());
        } else {
            output.push(
                serde_yaml_ng::to_string(&vec![entry])
                    .map_err(|_| Error::protocol("无法编码 DSH 覆盖层"))?,
            );
        }
    }
    Ok(output.concat())
}

fn yaml_expressions(value: &serde_yaml_ng::Value) -> Result<Value> {
    use serde_yaml_ng::Value as Yaml;
    match value {
        Yaml::Tagged(tagged) if tagged.tag == "tag:yaml.org,2002:js" || tagged.tag == "!js" => {
            let expression = tagged
                .value
                .as_str()
                .ok_or_else(|| Error::invalid("DSH !!js 表达式必须是字符串"))?;
            Ok(json!({"__jsExpr": expression}))
        }
        Yaml::Tagged(_) => Err(Error::invalid(
            "DSH 提供方使用了不支持的 YAML 标签，请使用普通值或 !!js",
        )),
        Yaml::Sequence(items) => items
            .iter()
            .map(yaml_expressions)
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        Yaml::Mapping(items) => items
            .iter()
            .map(|(key, value)| {
                let key = key
                    .as_str()
                    .ok_or_else(|| Error::invalid("DSH 提供方字段名必须是字符串"))?;
                Ok((key.to_owned(), yaml_expressions(value)?))
            })
            .collect::<Result<serde_json::Map<_, _>>>()
            .map(Value::Object),
        _ => serde_json::to_value(value).map_err(|_| Error::invalid("无法编码 DSH 配置值")),
    }
}
/// The config root the process runs in: the request's, else DSH's default `~/.dsh`, created when
/// missing so a fresh install starts.
fn dsh_home(request: &StartSession) -> Result<PathBuf> {
    let home = process::config_dir(request.config_root.as_deref(), ".dsh")?;
    std::fs::create_dir_all(&home).context(format!("无法创建 DSH 配置目录 {}", home.display()))?;
    Ok(home)
}

/// `session/new` / `session/resume` parameters: the host tool server (when on) followed by the
/// projected MCP servers, whose values travel inline (04 §6.4).
pub(super) fn session_params(request: &StartSession, cwd: &Path) -> Result<Value> {
    let mut servers = match &request.host_tools {
        Some(access) => vec![host::acp_server(access)?],
        None => Vec::new(),
    };
    let mut projected = projection_engine::acp_mcp_servers(&request.routing)?;
    for server in &mut projected {
        if let Some(command) = server["command"].as_str() {
            server["command"] = json!(mcp_executable(command)?);
        }
    }
    servers.extend(projected);
    let mut params = json!({"cwd": cwd, "mcpServers": servers});
    if let Some(key) = &request.resume {
        params["sessionId"] = json!(key.as_str());
    }
    Ok(params)
}

pub(super) fn mcp_executable(command: &str) -> Result<PathBuf> {
    let path = Path::new(command);
    if path.is_absolute() && path.is_file() {
        return Ok(process::plain_path(path));
    }
    if path.components().count() == 1 {
        let extensions =
            std::env::var_os("PATHEXT").unwrap_or_else(|| ".EXE;.CMD;.BAT;.COM".into());
        let extensions = extensions.to_string_lossy();
        if let Some(search) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&search) {
                if !directory.is_absolute() {
                    continue;
                }
                let candidate = directory.join(path);
                if candidate.is_file() {
                    return Ok(process::plain_path(&candidate));
                }
                #[cfg(windows)]
                if path.extension().is_none() {
                    for extension in extensions.split(';').filter(|value| !value.is_empty()) {
                        let candidate = directory.join(format!("{command}{extension}"));
                        if candidate.is_file() {
                            return Ok(process::plain_path(&candidate));
                        }
                    }
                }
            }
        }
    }
    Err(Error::invalid(format!(
        "找不到 MCP 命令 {command}，请填写可执行文件的绝对路径，或将它加入 PATH"
    )))
}

/// DSH fails session creation when one of its MCP servers cannot start; point at the settings.
pub(super) fn mcp_hint(error: Error, request: &StartSession) -> Error {
    match error {
        Error::Domain { kind, context } if !request.routing.mcp.is_empty() => {
            let names: Vec<&str> = request
                .routing
                .mcp
                .iter()
                .map(|s| s.name.as_str())
                .collect();
            Error::new(
                kind,
                format!(
                    "{context}；若是 MCP 服务器（{}）无法启动，请检查 设置 → MCP 服务器",
                    names.join("、")
                ),
            )
        }
        error => error,
    }
}
