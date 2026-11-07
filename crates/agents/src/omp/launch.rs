//! OMP launch arguments: CLI flags, generated extension and overlay, and the `mcp.json` sync.

use thinkrail_core::{Result, SessionRole, StartSession};

use crate::{
    host, omp_mcp,
    process::{self, LaunchSpec},
    projection_engine, workflow,
};

/// Host arguments and environment of one OMP launch: the extension (`-e`: workflow rules and
/// providers), the overlay (`--config`: bundled skills, MCP, default model) and the `thinkrail-`
pub(super) fn launch_spec(request: &StartSession) -> Result<LaunchSpec<'_>> {
    let reviewer = matches!(request.role, SessionRole::Reviewer { .. });
    let mut host_args: Vec<String> = ["--mode", "rpc-ui", "--approval-mode", "always-ask"]
        .into_iter()
        .map(Into::into)
        .collect();
    if !reviewer {
        omp_mcp::sync(
            &process::config_dir(request.config_root.as_deref(), ".omp/agent")?,
            &request.routing.mcp,
        )?;
    }
    let extension = if reviewer {
        projection_engine::omp_reviewer_extension(&request.routing)
    } else {
        projection_engine::omp_extension(&request.routing)
    };
    let path = host::write_support(
        &request.support_dir,
        projection_engine::OMP_EXTENSION_FILE,
        &extension,
    )?;
    host_args.extend(["-e".to_owned(), path.display().to_string()]);
    if reviewer {
        host_args.extend([
            "--no-session".into(),
            "--no-skills".into(),
            "--no-rules".into(),
            "--no-title".into(),
            "--tools".into(),
            "read,grep,glob,bash".into(),
            "--system-prompt".into(),
            match &request.role {
                SessionRole::Reviewer { system_prompt } => system_prompt.clone(),
                SessionRole::Worker => unreachable!(),
            },
        ]);
    } else {
        let skills = workflow::stage_skills(&request.support_dir)?;
        let overlay = projection_engine::omp_overlay(&request.routing, &skills);
        let path = host::write_support(
            &request.support_dir,
            projection_engine::OMP_OVERLAY_FILE,
            &overlay,
        )?;
        host_args.extend(["--config".to_owned(), path.display().to_string()]);
        if let Some(key) = &request.resume {
            host_args.extend(["--session".into(), key.as_str().to_owned()]);
        }
    }
    if reviewer && let Some(model) = projection_engine::model_flag(&request.routing) {
        host_args.extend(["--model".into(), model]);
    }
    Ok(LaunchSpec {
        launch: &request.launch,
        config_root: request.config_root.as_deref(),
        host_args,
        cwd: request.cwd.clone(),
        config_var: "PI_CODING_AGENT_DIR",
        host_env: process::routing_env(&request.routing).collect(),
    })
}
