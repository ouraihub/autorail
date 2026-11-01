//! Launch fixtures for the adapter `launch_spec` tests: a temporary config root, cwd and support
//! directory, host tool access and a routing that carries placeholder secrets.

use std::{ffi::OsString, fs, path::PathBuf, sync::Arc};

use serde_json::Value;
use tempfile::TempDir;
use thinkrail_core::{
    BackendKind, DefaultModel, EngineRouting, EnvBinding, Error, HostToolAccess, HostToolHandler, HostToolResult,
    HostToolSpec, Launch, PlanScope, ProjectedMcpServer, ProjectedMcpTransport, ProjectedModel,
    ProjectedProvider, Protocol, Result, Secret, StartSession, ThinkingLevel,
};

/// Placeholder secrets of [`secret_routing`]; none may appear in a generated file.
pub(crate) const SECRETS: [&str; 3] = ["sk-test-a", "sk-test-b", "sk-test-c"];

struct NoTools;

impl HostToolHandler for NoTools {
    fn specs(&self) -> Vec<HostToolSpec> {
        Vec::new()
    }

    fn call(&self, _scope: &PlanScope, name: &str, _arguments: Value) -> Result<HostToolResult> {
        Err(Error::invalid(format!("unexpected host tool call {name}")))
    }
}

pub(crate) struct Fixture {
    root: TempDir,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("config")).unwrap();
        fs::create_dir_all(root.path().join("cwd")).unwrap();
        Self { root }
    }

    pub(crate) fn config(&self) -> PathBuf {
        self.root.path().join("config")
    }

    pub(crate) fn cwd(&self) -> PathBuf {
        self.root.path().join("cwd")
    }

    pub(crate) fn support(&self) -> PathBuf {
        self.root.path().join("support")
    }

    pub(crate) fn exe(&self) -> PathBuf {
        self.root.path().join("thinkrail.exe")
    }

    /// `support/<name>` as it appears in a host argument.
    pub(crate) fn support_arg(&self, name: &str) -> String {
        self.support().join(name).display().to_string()
    }

    pub(crate) fn request(
        &self,
        backend: BackendKind,
        resume: Option<&str>,
        host_tools: bool,
        routing: EngineRouting,
    ) -> StartSession {
        let host_tools = host_tools.then(|| HostToolAccess {
            executable: self.exe(),
            pipe: "pipe-1".into(),
            token: "token-1".into(),
            handler: Arc::new(NoTools),
        });
        StartSession {
            chat: "chat".into(),
            workspace: "w".into(),
            backend,
            launch: Launch::External { executable: self.root.path().join("engine.exe"), args: vec!["cli.js".into()] },
            config_root: Some(self.config()),
            cwd: self.cwd(),
            resume: resume.map(Into::into),
            host_tools,
            routing,
            support_dir: self.support(),
        }
    }

    /// Fails if any file under the support directory (staged skills included) contains a
    /// placeholder secret.
    pub(crate) fn assert_no_secret_written(&self) {
        let mut dirs = vec![self.support()];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap();
                for secret in SECRETS {
                    assert!(!text.contains(secret), "{} contains {secret}", path.display());
                }
            }
        }
    }
}

/// A routing whose only content is environment: one provider key, one header and one MCP value.
pub(crate) fn secret_routing() -> EngineRouting {
    EngineRouting {
        env: vec![
            ("THINKRAIL_PROVIDER_1A2B3C4D_KEY".into(), Secret::new(SECRETS[0])),
            ("THINKRAIL_HDR_1A2B3C4D_0".into(), Secret::new(SECRETS[1])),
            ("THINKRAIL_MCP_5E6F7A8B_H0".into(), Secret::new(SECRETS[2])),
        ],
        ..EngineRouting::default()
    }
}

/// `EngineRouting.env` as the launch variables it must become.
pub(crate) fn routing_vars(routing: &EngineRouting) -> Vec<(String, OsString)> {
    routing.env.iter().map(|(name, value)| (name.clone(), OsString::from(value.expose()))).collect()
}

/// A full routing: an Anthropic provider with one header and a reasoning, vision model; an OpenAI
/// chat provider with a plain model; a default model with a level; one stdio and one HTTP MCP
/// server. Every value lives in `env` as a placeholder secret.
pub(crate) fn sample_routing() -> EngineRouting {
    let claude = ProjectedModel {
        id: "claude-x".into(),
        name: "Claude X".into(),
        vision: true,
        reasoning: true,
        efforts: vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::High],
        context_window: 200_000,
        max_tokens: 16_384,
    };
    let plain = ProjectedModel {
        id: "gpt-y".into(),
        name: "GPT \"Y\"".into(),
        vision: false,
        reasoning: false,
        efforts: Vec::new(),
        context_window: 128_000,
        max_tokens: 16_384,
    };
    let binding = |name: &str, var: &str| EnvBinding { name: name.into(), var: var.into() };
    EngineRouting {
        providers: vec![
            ProjectedProvider {
                route: "thinkrail-1a2b3c4d-am".into(),
                display_name: "中转 A".into(),
                protocol: Protocol::AnthropicMessages,
                base_url: "https://api.example.com".into(),
                key_var: "THINKRAIL_PROVIDER_1A2B3C4D_KEY".into(),
                headers: vec![binding("x-extra", "THINKRAIL_HDR_1A2B3C4D_0")],
                models: vec![claude],
            },
            ProjectedProvider {
                route: "thinkrail-9f8e7d6c-oc".into(),
                display_name: "OpenAI B".into(),
                protocol: Protocol::OpenaiChat,
                base_url: "https://b.example.com/v1".into(),
                key_var: "THINKRAIL_PROVIDER_9F8E7D6C_KEY".into(),
                headers: Vec::new(),
                models: vec![plain],
            },
        ],
        default_model: Some(DefaultModel {
            route: "thinkrail-1a2b3c4d-am".into(),
            model_id: "claude-x".into(),
            effort: Some(ThinkingLevel::High),
        }),
        mcp: vec![
            ProjectedMcpServer {
                name: "thinkrail-echo".into(),
                transport: ProjectedMcpTransport::Stdio {
                    command: "node".into(),
                    args: vec!["echo.js".into()],
                    env: vec![binding("ECHO_TOKEN", "THINKRAIL_MCP_5E6F7A8B_E0")],
                },
            },
            ProjectedMcpServer {
                name: "thinkrail-docs".into(),
                transport: ProjectedMcpTransport::Http {
                    url: "https://mcp.example.com/mcp".into(),
                    headers: vec![binding("Authorization", "THINKRAIL_MCP_0A0B0C0D_H0")],
                },
            },
        ],
        env: vec![
            ("THINKRAIL_PROVIDER_1A2B3C4D_KEY".into(), Secret::new(SECRETS[0])),
            ("THINKRAIL_HDR_1A2B3C4D_0".into(), Secret::new(SECRETS[1])),
            ("THINKRAIL_PROVIDER_9F8E7D6C_KEY".into(), Secret::new("thinkrail-no-key")),
            ("THINKRAIL_MCP_5E6F7A8B_E0".into(), Secret::new(SECRETS[2])),
            ("THINKRAIL_MCP_0A0B0C0D_H0".into(), Secret::new(format!("Bearer {}", SECRETS[0]))),
        ],
        signature: "sig".into(),
    }
}

/// [`sample_routing`] as `prepare` builds it for DSH: providers with headers go through the
/// gateway, so no provider carries header bindings.
pub(crate) fn dsh_routing() -> EngineRouting {
    let mut routing = sample_routing();
    for provider in &mut routing.providers {
        provider.headers.clear();
    }
    routing.env.retain(|(name, _)| !name.starts_with("THINKRAIL_HDR_"));
    routing
}
