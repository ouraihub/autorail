//! Unit tests of the engine launch environment, path handling and stderr redaction.

use super::*;

#[test]
fn credentials_and_engine_config_are_never_inherited() {
    for name in [
        "OPENAI_API_KEY",
        "my_token",
        "PI_CODING_AGENT_DIR",
        "dsh_home",
        "AWS_PROFILE",
        "ANTHROPIC_BASE_URL",
        "GITHUB_TOKEN",
        "CUSTOM_BASE_URL",
        "THINKRAIL_HOST_TOKEN",
    ] {
        assert!(is_isolated(name), "{name}");
    }
    for name in [
        "PATH",
        "SystemRoot",
        "USERPROFILE",
        "APPDATA",
        "TEMP",
        "NODE_OPTIONS",
    ] {
        assert!(!is_isolated(name), "{name}");
    }
}

#[test]
fn stderr_values_after_sensitive_words_are_masked() {
    let text = "boot ok\nOPENAI_API_KEY=sk-live-1\nAuthorization: Bearer abc\nInvalid secret sk-proj-0123456789abcdef.\n中文 password：x";
    let masked = redact(text);
    assert!(
        !masked.contains("sk-live-1") && !masked.contains("abc") && !masked.contains("0123456789"),
        "{masked}"
    );
    assert!(!masked.ends_with('x'), "{masked}");
    assert!(
        masked.starts_with("boot ok\nOPENAI_API_KEY= ***"),
        "{masked}"
    );
    assert_eq!(masked.lines().count(), 5);
}

#[test]
fn credential_prose_and_links_stay_readable() {
    let text = "No API key found for unknown.\nUse /login into a provider via OAuth or API key. See: D:\\pi\\docs\\providers.md";
    assert_eq!(redact(text), text);
}

#[test]
fn verbatim_prefixes_are_stripped_for_engines() {
    assert_eq!(
        plain_path(Path::new(r"\\?\C:\work\a")),
        PathBuf::from(r"C:\work\a")
    );
    assert_eq!(
        plain_path(Path::new(r"\\?\UNC\srv\share")),
        PathBuf::from(r"\\srv\share")
    );
    assert_eq!(plain_path(Path::new(r"C:\work")), PathBuf::from(r"C:\work"));
}

#[test]
fn routing_env_is_applied_as_launch_variables() {
    let f = crate::testing::Fixture::new();
    let request = f.request(
        thinkrail_core::BackendKind::Pi,
        None,
        false,
        crate::testing::secret_routing(),
    );
    let Launch::External { executable, .. } = &request.launch;
    std::fs::write(executable, b"").unwrap();
    let host_env = routing_env(&request.routing).collect();
    let spec = LaunchSpec {
        launch: &request.launch,
        host_args: Vec::new(),
        cwd: f.cwd(),
        config_root: request.config_root.as_deref(),
        config_var: "PI_CODING_AGENT_DIR",
        host_env,
    };
    let command = command(&spec).unwrap();
    let envs: Vec<_> = command
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(ToOwned::to_owned)))
        .collect();
    for (name, value) in crate::testing::routing_vars(&request.routing) {
        assert!(envs.contains(&(name.clone().into(), Some(value))), "{name}");
    }
}
