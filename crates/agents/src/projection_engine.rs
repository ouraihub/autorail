//! Engine projections of an [`EngineRouting`] (agent-settings 04 §6): the pi extension, the OMP
//! extension and overlay, the DSH patch and ACP `mcpServers`, plus the shared spellings (protocol
//! `api` names, thinking-level maps). Pure functions. Generated files reference variable names
//! only; [`acp_mcp_servers`] is the one place values are read, and its output travels over stdin.
//! Not to be confused with [`crate::projection`], the session snapshot projection.

use serde_json::{Value, json};
use thinkrail_core::{
    EngineRouting, EnvBinding, Error, ProjectedMcpServer, ProjectedMcpTransport, ProjectedModel,
    ProjectedProvider, Protocol, Result, ThinkingLevel,
};

/// Generated file names under `StartSession.support_dir` (04 §6.2–6.4).
pub(crate) const PI_EXTENSION_FILE: &str = "pi-thinkrail.js";
pub(crate) const OMP_EXTENSION_FILE: &str = "omp-thinkrail.ts";
pub(crate) const OMP_OVERLAY_FILE: &str = "omp-thinkrail.yml";
pub(crate) const DSH_PATCH_FILE: &str = "dsh-thinkrail.patch.yml";

/// Every level in ascending order; generated maps follow this order.
pub(crate) const LEVELS: [ThinkingLevel; 7] = [
    ThinkingLevel::Off,
    ThinkingLevel::Minimal,
    ThinkingLevel::Low,
    ThinkingLevel::Medium,
    ThinkingLevel::High,
    ThinkingLevel::Xhigh,
    ThinkingLevel::Max,
];

/// One level of a generated map: the level's name and the value sent for it (`None` = `null`).
pub(crate) type LevelEntry = (&'static str, Option<&'static str>);

/// The engine `api` name of a protocol (identical for pi, OMP and DSH).
pub(crate) fn api_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenaiChat => "openai-completions",
        Protocol::OpenaiResponses => "openai-responses",
        Protocol::AnthropicMessages => "anthropic-messages",
    }
}

/// The level as the engines spell it (`--thinking`, `modelRoles`, map keys and values).
pub(crate) fn level_name(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "off",
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::Xhigh => "xhigh",
        ThinkingLevel::Max => "max",
    }
}

/// pi `thinkingLevelMap`: every level, declared ones mapped to themselves and the rest `null`.
/// pi treats a missing key as "provider default" (it would send `medium`), so undeclared levels
/// must be explicit `null` ("unsupported"). `None` for a model that does not reason.
pub(crate) fn pi_thinking_level_map(model: &ProjectedModel) -> Option<Vec<LevelEntry>> {
    model.reasoning.then(|| {
        LEVELS
            .iter()
            .map(|&level| {
                (
                    level_name(level),
                    model.efforts.contains(&level).then(|| level_name(level)),
                )
            })
            .collect()
    })
}

/// DSH `reasoningEfforts`: only the declared levels, `off` mapped to `null` (reasoning disabled).
/// `None` for a model that does not reason (written as `reasoningEfforts: false`).
pub(crate) fn dsh_reasoning_efforts(model: &ProjectedModel) -> Option<Vec<LevelEntry>> {
    model.reasoning.then(|| {
        LEVELS
            .iter()
            .filter(|level| model.efforts.contains(level))
            .map(|&level| {
                (
                    level_name(level),
                    (level != ThinkingLevel::Off).then(|| level_name(level)),
                )
            })
            .collect()
    })
}

/// `text` as a double-quoted literal, valid in JavaScript, TypeScript, JSON and YAML alike.
fn quote(text: &str) -> String {
    Value::from(text).to_string()
}

/// `[a, b]` of quoted strings.
fn list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    let items: Vec<String> = items.into_iter().map(quote).collect();
    format!("[{}]", items.join(", "))
}

/// `{ "k": v, … }` with the keys quoted and the values already rendered; `{}` when empty.
fn object(entries: impl IntoIterator<Item = (String, String)>) -> String {
    let entries: Vec<String> = entries
        .into_iter()
        .map(|(key, value)| format!("{}: {value}", quote(&key)))
        .collect();
    if entries.is_empty() {
        "{}".to_owned()
    } else {
        format!("{{ {} }}", entries.join(", "))
    }
}

/// A level map: `null` for `None`, else the quoted value.
fn level_map(entries: &[LevelEntry]) -> String {
    object(entries.iter().map(|(key, value)| {
        (
            (*key).to_owned(),
            value.map_or_else(|| "null".to_owned(), quote),
        )
    }))
}

fn input(model: &ProjectedModel) -> String {
    list(
        if model.vision {
            &["text", "image"][..]
        } else {
            &["text"][..]
        }
        .iter()
        .copied(),
    )
}

/// Default model as the engines spell it: `<route>/<model id>`.
fn model_ref(route: &str, model_id: &str) -> String {
    format!("{route}/{model_id}")
}

/// The host tool server of the pi extension. `exposure: "direct"` declares the tools to the model
/// (the default codemode hides them); the per-request `timeout` of 1200 s outlasts a plan-step
/// review (15 minutes, agent-settings 08 §5.5). The child inherits pi's environment, which carries
/// `THINKRAIL_HOST_PIPE` / `THINKRAIL_HOST_TOKEN`.
const PI_HOST_TOOLS: &str = r#"	pi.registerMcpServer("thinkrail", {
		command: process.env.THINKRAIL_HOST_EXE,
		args: ["mcp-host"],
		exposure: "direct",
		timeout: 1200,
		description: "ThinkRail host: this chat's TODO plan and review comments.",
	});
"#;

/// `${VAR}`: pi resolves it once from its environment. Used for every MCP value rather than
/// `process.env.VAR`: pi re-parses MCP `env` / `headers` as templates, so a resolved value holding
/// `$X` or a leading `!` would be expanded or run as a command.
fn pi_reference(var: &str) -> String {
    quote(&format!("${{{var}}}"))
}

fn pi_bindings(bindings: &[EnvBinding]) -> String {
    object(
        bindings
            .iter()
            .map(|b| (b.name.clone(), pi_reference(&b.var))),
    )
}

fn pi_model(model: &ProjectedModel) -> String {
    let mut fields = vec![
        format!("id: {}", quote(&model.id)),
        format!("name: {}", quote(&model.name)),
        format!("reasoning: {}", model.reasoning),
        format!("input: {}", input(model)),
    ];
    if let Some(map) = pi_thinking_level_map(model) {
        fields.push(format!("thinkingLevelMap: {}", level_map(&map)));
    }
    fields.push("cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }".to_owned());
    fields.push(format!("contextWindow: {}", model.context_window));
    fields.push(format!("maxTokens: {}", model.max_tokens));
    format!("{{ {} }}", fields.join(", "))
}

fn pi_provider(provider: &ProjectedProvider) -> String {
    let models: Vec<String> = provider
        .models
        .iter()
        .map(|m| format!("\t\t\t{},\n", pi_model(m)))
        .collect();
    let mut out = format!(
        "\tpi.registerProvider({}, {{\n\t\tname: {},\n\t\tbaseUrl: {},\n\t\tapi: {},\n\t\tapiKey: {},\n",
        quote(&provider.route),
        quote(&provider.display_name),
        quote(&provider.base_url),
        quote(api_name(provider.protocol)),
        quote(&format!("${}", provider.key_var)),
    );
    if !provider.headers.is_empty() {
        out.push_str(&format!(
            "\t\theaders: {},\n",
            pi_bindings(&provider.headers)
        ));
    }
    out.push_str(&format!(
        "\t\tmodels: [\n{}\t\t],\n\t}});\n",
        models.concat()
    ));
    out
}

fn pi_mcp(server: &ProjectedMcpServer) -> String {
    let config = match &server.transport {
        ProjectedMcpTransport::Stdio { command, args, env } => {
            let mut fields = vec![
                format!("command: {}", quote(command)),
                format!("args: {}", list(args.iter().map(String::as_str))),
            ];
            if !env.is_empty() {
                fields.push(format!("env: {}", pi_bindings(env)));
            }
            fields
        }
        ProjectedMcpTransport::Http { url, headers } => {
            let mut fields = vec![format!("url: {}", quote(url))];
            if !headers.is_empty() {
                fields.push(format!("headers: {}", pi_bindings(headers)));
            }
            fields
        }
    };
    format!(
        "\tpi.registerMcpServer({}, {{ {} }});\n",
        quote(&server.name),
        config.join(", ")
    )
}

/// The original extensions' `before_agent_start` hook: the workflow and spec rules as named
/// system-prompt sections (pi `systemPromptOptions.sections`).
fn pi_rules() -> String {
    let sections: Vec<String> = crate::workflow::RULES
        .iter()
        .map(|(name, rule)| {
            format!(
                "\t\tevent.systemPromptOptions.sections[{}] = {};\n",
                quote(name),
                quote(rule)
            )
        })
        .collect();
    format!(
        "\tpi.on(\"before_agent_start\", (event) => {{\n{}\t}});\n",
        sections.concat()
    )
}

/// `pi-thinkrail.js` (04 §6.2): the workflow rules, host tools (when on), then providers, then
/// MCP servers.
pub(crate) fn pi_extension(host_tools: bool, routing: &EngineRouting) -> String {
    let mut body = String::from("export default function (pi) {\n");
    body.push_str(&pi_rules());
    if host_tools {
        body.push_str(PI_HOST_TOOLS);
    }
    body.extend(routing.providers.iter().map(pi_provider));
    body.extend(routing.mcp.iter().map(pi_mcp));
    body.push_str("}\n");
    body
}

/// A reviewer's pi extension: the providers only. No workflow rules (its system prompt is the
/// reviewer's), no host tools, no MCP servers (08 §5.4).
pub(crate) fn pi_reviewer_extension(routing: &EngineRouting) -> String {
    let mut body = String::from("export default function (pi) {\n");
    body.extend(routing.providers.iter().map(pi_provider));
    body.push_str("}\n");
    body
}

/// pi arguments for the default model (new sessions only; a resumed session keeps its own).
pub(crate) fn pi_model_args(routing: &EngineRouting) -> Vec<String> {
    let Some(default) = &routing.default_model else {
        return Vec::new();
    };
    let mut args = vec![
        "--model".to_owned(),
        model_ref(&default.route, &default.model_id),
    ];
    if let Some(effort) = default.effort {
        args.extend(["--thinking".to_owned(), level_name(effort).to_owned()]);
    }
    args
}

/// The extension API surface `omp-thinkrail.ts` uses; the rest of OMP's API stays untyped.
const OMP_EXTENSION_HEAD: &str = "interface ExtensionApi {\n\tregisterProvider(name: string, config: Record<string, unknown>): void;\n\ton(event: \"before_agent_start\", handler: (event: { systemPrompt: string[] }) => { systemPrompt: string[] } | void): void;\n}\n\nexport default function (pi: ExtensionApi): void {\n";

/// OMP model fields. `thinking` is left out on purpose: OMP passes it through verbatim and a
/// range without `mode` disables its adaptive / effort request paths, while an omitted
/// `thinking` lets OMP derive mode and range from the model identity and `api` (as for a
/// `models.yml` entry). OMP clamps the requested level to that range.
fn omp_model(model: &ProjectedModel) -> String {
    format!(
        "{{ id: {}, name: {}, reasoning: {}, input: {}, contextWindow: {}, maxTokens: {} }}",
        quote(&model.id),
        quote(&model.name),
        model.reasoning,
        input(model),
        model.context_window,
        model.max_tokens
    )
}

/// `apiKey` is the bare variable name (OMP resolves it from its environment); `auth: "apiKey"` or
/// OMP builds `anthropic-messages` requests the OAuth way. Header values are read when the
/// extension loads and live only in OMP's memory.
fn omp_provider(provider: &ProjectedProvider) -> String {
    let models: Vec<String> = provider
        .models
        .iter()
        .map(|m| format!("\t\t\t{},\n", omp_model(m)))
        .collect();
    let mut out = format!(
        "\tpi.registerProvider({}, {{\n\t\tbaseUrl: {},\n\t\tapi: {},\n\t\tapiKey: {},\n\t\tauth: \"apiKey\",\n\t\tauthHeader: true,\n",
        quote(&provider.route),
        quote(&provider.base_url),
        quote(api_name(provider.protocol)),
        quote(&provider.key_var),
    );
    if !provider.headers.is_empty() {
        let headers = object(
            provider
                .headers
                .iter()
                .map(|b| (b.name.clone(), format!("process.env.{} ?? \"\"", b.var))),
        );
        out.push_str(&format!("\t\theaders: {headers},\n"));
    }
    out.push_str(&format!(
        "\t\tmodels: [\n{}\t\t],\n\t}});\n",
        models.concat()
    ));
    out
}

/// `omp-thinkrail.ts` (04 §6.3): the workflow rules appended to OMP's system prompt
/// (`before_agent_start` → `systemPrompt`), then provider registrations (host tools travel over RPC).
pub(crate) fn omp_extension(routing: &EngineRouting) -> String {
    let mut body = OMP_EXTENSION_HEAD.to_owned();
    let rules: Vec<String> = crate::workflow::RULES
        .iter()
        .map(|(name, rule)| quote(&format!("<{name}>\n{rule}\n</{name}>")))
        .collect();
    body.push_str(&format!(
        "\tpi.on(\"before_agent_start\", (event) => ({{ systemPrompt: [...event.systemPrompt, {}] }}));\n",
        rules.join(", ")
    ));
    body.extend(routing.providers.iter().map(omp_provider));
    body.push_str("}\n");
    body
}

/// A reviewer's OMP extension: the providers only, no workflow rules (08 §5.4).
pub(crate) fn omp_reviewer_extension(routing: &EngineRouting) -> String {
    let mut body = OMP_EXTENSION_HEAD.to_owned();
    body.extend(routing.providers.iter().map(omp_provider));
    body.push_str("}\n");
    body
}

/// `provider/model[:level]`, the `--model` value of a reviewer's pi/OMP launch.
pub(crate) fn model_flag(routing: &EngineRouting) -> Option<String> {
    let default = routing.default_model.as_ref()?;
    let model = model_ref(&default.route, &default.model_id);
    Some(match default.effort {
        Some(effort) => format!("{model}:{}", level_name(effort)),
        None => model,
    })
}

/// `omp-thinkrail.yml` (04 §6.3), loaded with `--config`: the bundled skills directory, project
/// MCP configs off when there are host MCP servers, and the default model as `modelRoles.default`.
pub(crate) fn omp_overlay(routing: &EngineRouting, skills: &std::path::Path) -> String {
    let mut body = format!(
        "skills:\n  customDirectories:\n    - {}\n",
        quote(&skills.display().to_string())
    );
    if !routing.mcp.is_empty() || routing.default_model.is_some() {
        body.push_str("mcp:\n  enableProjectConfig: false\n");
    }
    if let Some(default) = &routing.default_model {
        let mut role = model_ref(&default.route, &default.model_id);
        if let Some(effort) = default.effort {
            role = format!("{role}:{}", level_name(effort));
        }
        body.push_str(&format!("modelRoles:\n  default: {}\n", quote(&role)));
    }
    body
}

fn dsh_model(model: &ProjectedModel) -> String {
    let efforts =
        dsh_reasoning_efforts(model).map_or_else(|| "false".to_owned(), |map| level_map(&map));
    format!(
        "          - id: {}\n            name: {}\n            contextWindow: {}\n            maxTokens: {}\n            input: {}\n            reasoningEfforts: {efforts}\n",
        quote(&model.id),
        quote(&model.name),
        model.context_window,
        model.max_tokens,
        input(model)
    )
}

/// `@deepseek-ai/dsh-acp-app` 0.2.1-rc.5 `system-prompt.personaPrefix`, restated in our patch row.
const DSH_ACP_PERSONA: &str = "You are a coding agent powered by the {{model}} model.";

/// `dsh-thinkrail.patch.yml` (04 §6.4): the bundled skills directory, the workflow rules as the
/// persona suffix (the `acp` persona prefix is restated: a patch row replaces its whole config),
/// the built-in TODO tool off (host tools on), the providers merged into `llm-pi-ai`, and the
/// `acp` default model. Providers carry no headers: DSH cannot reference header values by
/// variable, so `prepare` routes such providers through the gateway.
pub(crate) fn dsh_patch(
    host_tools: bool,
    routing: &EngineRouting,
    skills: &std::path::Path,
) -> Result<String> {
    let mut body = format!(
        "- id: skill-filesystem\n  config:\n    customSkillDirs:\n      - {}\n",
        quote(&skills.display().to_string())
    );
    let rules: Vec<String> = crate::workflow::RULES
        .iter()
        .map(|(name, rule)| format!("<{name}>\n{rule}\n</{name}>"))
        .collect();
    body.push_str(&format!(
        "- id: system-prompt\n  config:\n    personaPrefix: {}\n    personaSuffix: {}\n",
        quote(DSH_ACP_PERSONA),
        quote(&rules.join("\n\n"))
    ));
    if host_tools {
        body.push_str("- id: tool-todo\n  disabled: true\n");
    }
    dsh_routing(&mut body, routing)?;
    Ok(body)
}

/// DSH 0.2.1-rc.5 (`dsh-base/cordis.patch.yml`) rows a plan-step reviewer must not have: writing
/// tools, TODO/goal state, skills, workflows, subagents, the web and plan mode (08 §5.4).
const DSH_REVIEWER_OFF: [&str; 10] = [
    "tool-todo",
    "tool-goal",
    "tool-skill",
    "tool-workflow",
    "tool-subagent",
    "tool-subagent-fork",
    "tool-subagent-control",
    "tool-subagent-list-agents",
    "tool-web",
    "plan-mode",
];

/// A reviewer's patch: `tool-fs` without `write`/`edit`, the rows above off, `prompt` as the
/// persona and the same providers and model as a worker. No skills and no host tools.
pub(crate) fn dsh_reviewer_patch(prompt: &str, routing: &EngineRouting) -> Result<String> {
    let mut body = String::from("- id: tool-fs\n  config:\n    write: false\n    edit: false\n");
    for row in DSH_REVIEWER_OFF {
        body.push_str(&format!("- id: {row}\n  disabled: true\n"));
    }
    body.push_str(&format!(
        "- id: system-prompt\n  config:\n    personaPrefix: {}\n",
        quote(prompt)
    ));
    dsh_routing(&mut body, routing)?;
    Ok(body)
}

/// The providers merged into `llm-pi-ai` and the `acp` default model.
fn dsh_routing(body: &mut String, routing: &EngineRouting) -> Result<()> {
    if !routing.providers.is_empty() {
        body.push_str("- id: llm-pi-ai\n  config:\n    providers:\n");
        for provider in &routing.providers {
            if !provider.headers.is_empty() {
                return Err(Error::invalid(format!(
                    "DSH 不能投影提供方 {} 的额外请求头，应改走网关",
                    provider.display_name
                )));
            }
            body.push_str(&format!(
                "      {}:\n        displayName: {}\n        api: {}\n        baseURL: {}\n        apiKeyEnv: {}\n        models:\n",
                quote(&provider.route),
                quote(&provider.display_name),
                quote(api_name(provider.protocol)),
                quote(&provider.base_url),
                quote(&provider.key_var),
            ));
            body.extend(provider.models.iter().map(dsh_model));
        }
    }
    if let Some(default) = &routing.default_model {
        body.push_str(&format!(
            "- id: acp\n  config:\n    provider: {}\n    model: {}\n",
            quote(&default.route),
            quote(&default.model_id)
        ));
    }
    Ok(())
}

fn acp_values(server: &str, bindings: &[EnvBinding], routing: &EngineRouting) -> Result<Value> {
    bindings
        .iter()
        .map(|binding| {
            let value = routing
                .env
                .iter()
                .find(|(name, _)| *name == binding.var)
                .map(|(_, value)| value.expose())
                .ok_or_else(|| {
                    Error::invalid(format!("MCP 服务器 {server} 缺少变量 {} 的值", binding.var))
                })?;
            Ok(json!({"name": binding.name, "value": value}))
        })
        .collect::<Result<Vec<_>>>()
        .map(Value::Array)
}

/// ACP `mcpServers` entries for DSH `session/new` / `session/resume` (04 §6.4). DSH scrubs its
/// parent's environment, so the values are taken from `routing.env` by `EnvBinding.var` and travel
/// in the JSON-RPC parameters (stdin only; never written to disk or logged).
pub(crate) fn acp_mcp_servers(routing: &EngineRouting) -> Result<Vec<Value>> {
    routing
        .mcp
        .iter()
        .map(|server| match &server.transport {
            ProjectedMcpTransport::Stdio { command, args, env } => Ok(json!({
                "name": server.name,
                "command": command,
                "args": args,
                "env": acp_values(&server.name, env, routing)?,
            })),
            ProjectedMcpTransport::Http { url, headers } => Ok(json!({
                "type": "http",
                "name": server.name,
                "url": url,
                "headers": acp_values(&server.name, headers, routing)?,
            })),
        })
        .collect()
}

#[cfg(test)]
mod tests;
