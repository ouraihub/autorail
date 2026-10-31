use std::{fmt::Debug, fs, path::Path, sync::Arc, thread};

use tempfile::TempDir;
use thinkrail_core::{
    Access, Endpoint, EnvValue, ErrorKind, Header, McpOrigin, McpScope, McpServer, McpServerInput, McpService,
    McpTransport, ModelPrefService, ModelRef, ProjectId, Protocol, Provider, ProviderId, ProviderInput,
    ProviderKeyId, ProviderKeyInput, ProviderKeyUpdate, ProviderModel, ProviderService, Result, Secret,
    ThinkingLevel,
};

use crate::{SqliteSettingsStore, mask::mask};

const DB: &str = "thinkrail.db";

fn open(directory: &TempDir) -> SqliteSettingsStore {
    SqliteSettingsStore::open(&directory.path().join("data").join(DB)).unwrap()
}

#[track_caller]
fn kind<T: Debug>(result: Result<T>, expected: ErrorKind) {
    let error = result.unwrap_err();
    assert_eq!(error.kind(), Some(expected), "{error}");
}

fn model(id: &str, protocols: &[Protocol]) -> ProviderModel {
    ProviderModel {
        model_id: id.into(),
        name: id.to_uppercase(),
        protocols: protocols.to_vec(),
        context_window: Some(200_000),
        max_tokens: None,
        vision: true,
        reasoning: true,
        tools: true,
        efforts: vec![ThinkingLevel::Low, ThinkingLevel::High],
        enabled: true,
    }
}

fn input(name: &str) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        endpoints: vec![
            Endpoint { protocol: Protocol::OpenaiChat, base_url: "https://relay.example/v1".into() },
            Endpoint { protocol: Protocol::AnthropicMessages, base_url: "https://relay.example".into() },
        ],
        default_protocol: Protocol::OpenaiChat,
        headers: vec![Header { name: "X-Trace".into(), value: "trace-value".into() }],
        access: Access::Direct,
        enabled: true,
        models: vec![
            model("gpt-test", &[Protocol::OpenaiChat]),
            model("claude-test", &[Protocol::AnthropicMessages, Protocol::OpenaiChat]),
        ],
    }
}

fn key(value: &str) -> ProviderKeyInput {
    ProviderKeyInput { label: None, key: Secret::new(value), enabled: true }
}

fn toggle(enabled: bool) -> ProviderKeyUpdate {
    ProviderKeyUpdate { label: None, key: None, enabled }
}

fn create(store: &SqliteSettingsStore, input: ProviderInput) -> Result<Provider> {
    ProviderService::create(store, input)
}

fn update(store: &SqliteSettingsStore, provider: &Provider, input: ProviderInput) -> Result<Provider> {
    ProviderService::update(store, &provider.id, provider.revision, input)
}

fn count(store: &SqliteSettingsStore, sql: &str, id: &str) -> i64 {
    store.read(|conn| conn.int(sql, (id,))).unwrap()
}

/// Rows that still reference provider `id` in each child table.
fn children(store: &SqliteSettingsStore, id: &ProviderId) -> [i64; 4] {
    ["provider_endpoints", "provider_models", "provider_keys", "hidden_models"]
        .map(|table| count(store, &format!("SELECT COUNT(*) FROM {table} WHERE provider_id = ?1"), id.as_str()))
}

fn send_sync<T: Send + Sync>() {}

/// 04 §3.1: one `turso::Database` shared by threads, each opening its own connection.
#[test]
fn turso_database_is_shared_across_threads() {
    send_sync::<turso::Database>();
    send_sync::<SqliteSettingsStore>();
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("probe.db");
    let db = Arc::new(pollster::block_on(turso::Builder::new_local(path.to_str().unwrap()).build()).unwrap());
    pollster::block_on(async {
        let conn = db.connect().unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, who TEXT NOT NULL)", ()).await.unwrap();
    });
    let workers: Vec<_> = (0..2)
        .map(|worker| {
            let db = Arc::clone(&db);
            thread::spawn(move || {
                pollster::block_on(async {
                    let conn = db.connect().unwrap();
                    conn.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
                    for row in 0..50 {
                        conn.execute("INSERT INTO t (who) VALUES (?1)", (format!("{worker}-{row}"),)).await.unwrap();
                    }
                })
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let count = pollster::block_on(async {
        let conn = db.connect().unwrap();
        let mut rows = conn.query("SELECT COUNT(*) FROM t", ()).await.unwrap();
        rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
    });
    assert_eq!(count, 100);
}

#[test]
fn store_writes_from_several_threads() {
    let directory = TempDir::new().unwrap();
    let store = Arc::new(open(&directory));
    let workers: Vec<_> = (0..4)
        .map(|worker| {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                for index in 0..5 {
                    let provider = create(&store, input(&format!("p{worker}-{index}"))).unwrap();
                    store.add_key(&provider.id, key(&format!("sk-test-{worker}-{index}"))).unwrap();
                    store.providers().unwrap();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let providers = store.providers().unwrap();
    assert_eq!(providers.len(), 20);
    assert!(providers.iter().all(|provider| provider.revision == 2 && provider.keys.len() == 1));
}

#[test]
fn provider_crud_with_name_and_revision_conflicts() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    assert!(store.providers().unwrap().is_empty());
    let mut padded = input("beta");
    padded.name = "  beta  ".into();
    padded.endpoints.reverse();
    let beta = create(&store, padded).unwrap();
    assert_eq!(beta.revision, 1);
    assert_eq!(beta.input, input("beta"), "name trimmed, endpoints in canonical order");
    assert!(beta.keys.is_empty());
    let alpha = create(&store, input("alpha")).unwrap();
    let names: Vec<_> = store.providers().unwrap().into_iter().map(|provider| provider.input.name).collect();
    assert_eq!(names, ["beta", "alpha"], "listed by creation sort, not by name");
    assert_eq!(store.provider(&beta.id).unwrap(), beta);

    kind(create(&store, input("beta")), ErrorKind::Conflict);
    kind(update(&store, &alpha, input("beta")), ErrorKind::Conflict);

    let mut changed = input("beta-renamed");
    changed.access = Access::Gateway;
    changed.models.truncate(1);
    let renamed = update(&store, &beta, changed.clone()).unwrap();
    assert_eq!((renamed.revision, &renamed.input), (2, &changed));
    assert_eq!(store.provider(&beta.id).unwrap(), renamed);
    kind(update(&store, &beta, input("stale")), ErrorKind::Conflict);
    kind(ProviderService::delete(&store, &beta.id, beta.revision), ErrorKind::Conflict);
    assert_eq!(store.provider(&beta.id).unwrap(), renamed, "conflicts change nothing");

    ProviderService::delete(&store, &renamed.id, renamed.revision).unwrap();
    kind(store.provider(&renamed.id), ErrorKind::NotFound);
    kind(update(&store, &renamed, input("gone")), ErrorKind::NotFound);
    assert_eq!(store.providers().unwrap(), vec![alpha]);
}

/// One invalid edit of an otherwise valid provider input.
type Breakage = (&'static str, fn(&mut ProviderInput));

#[test]
fn provider_validation() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let cases: [Breakage; 16] = [
        ("blank name", |input| input.name = "   ".into()),
        ("long name", |input| input.name = "n".repeat(65)),
        ("control in name", |input| input.name = "a\u{7}b".into()),
        ("no endpoints", |input| input.endpoints.clear()),
        ("ftp url", |input| input.endpoints[0].base_url = "ftp://relay.example".into()),
        ("userinfo", |input| input.endpoints[0].base_url = "https://user:pw@relay.example".into()),
        ("duplicate protocol", |input| input.endpoints[1].protocol = Protocol::OpenaiChat),
        ("default without endpoint", |input| input.default_protocol = Protocol::OpenaiResponses),
        ("reserved header", |input| input.headers[0].name = "Authorization".into()),
        ("bad header name", |input| input.headers[0].name = "bad header".into()),
        ("blank model id", |input| input.models[0].model_id = " ".into()),
        ("duplicate model", |input| input.models[1].model_id = "gpt-test".into()),
        ("model without protocol", |input| input.models[0].protocols.clear()),
        ("model protocol unset", |input| input.models[0].protocols = vec![Protocol::OpenaiResponses]),
        ("efforts without reasoning", |input| input.models[0].reasoning = false),
        ("too many models", |input| {
            input.models = (0..501).map(|i| model(&format!("m{i}"), &[Protocol::OpenaiChat])).collect();
        }),
    ];
    for (case, change) in cases {
        let mut bad = input("bad");
        change(&mut bad);
        let error = create(&store, bad).unwrap_err();
        assert_eq!(error.kind(), Some(ErrorKind::Invalid), "{case}: {error}");
        assert!(!error.to_string().contains("trace-value"), "{case}: header value leaked");
    }
    let mut fragment = input("fragment");
    fragment.endpoints[0].base_url = "https://relay.example/v1#".into();
    create(&store, fragment).unwrap();
    assert_eq!(store.providers().unwrap().len(), 1);
}

#[test]
fn update_keeps_keys_and_delete_cascades() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let provider = create(&store, input("relay")).unwrap();
    let other = create(&store, input("other")).unwrap();
    store.add_key(&other.id, key("sk-test-other")).unwrap();
    store.add_key(&provider.id, key("sk-test-a")).unwrap();
    let with_keys = store.add_key(&provider.id, key("sk-test-b")).unwrap();
    let gpt = ModelRef { provider: provider.id.clone(), model_id: "gpt-test".into() };
    let claude = ModelRef { provider: provider.id.clone(), model_id: "claude-test".into() };
    store.set_hidden(&gpt, true).unwrap();
    store.set_hidden(&claude, true).unwrap();

    let mut fewer = input("relay");
    fewer.models.retain(|model| model.model_id == "claude-test");
    let updated = update(&store, &with_keys, fewer).unwrap();
    assert_eq!(updated.keys, with_keys.keys, "update keeps keys");
    assert_eq!(store.prefs().unwrap().hidden, vec![claude], "hidden row of the removed model is gone");
    assert_eq!(children(&store, &provider.id), [2, 1, 2, 1]);

    ProviderService::delete(&store, &provider.id, updated.revision).unwrap();
    assert_eq!(children(&store, &provider.id), [0, 0, 0, 0]);
    assert!(store.prefs().unwrap().hidden.is_empty());
    assert_eq!(store.provider(&other.id).unwrap().keys.len(), 1, "other providers untouched");
}

#[test]
fn key_mask_boundaries() {
    assert_eq!(mask("sk-12345"), "••••••••", "8 characters stay hidden");
    assert_eq!(mask("sk-123456"), "sk****3456", "9 characters show 2 + 4");
    assert_eq!(mask(""), "••••••••");
    assert_eq!(mask("密钥一二三四五六七"), "密钥****四五六七", "counted in characters, not bytes");
}

#[test]
fn keys_are_validated_masked_and_revealed() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let provider = create(&store, input("relay")).unwrap();
    kind(store.add_key(&provider.id, key("   ")), ErrorKind::Invalid);
    let long_label = ProviderKeyInput { label: Some("l".repeat(65)), ..key("sk-test-label") };
    kind(store.add_key(&provider.id, long_label), ErrorKind::Invalid);
    kind(store.add_key(&ProviderId::from("missing"), key("sk-test-a")), ErrorKind::NotFound);

    let labelled = ProviderKeyInput { label: Some("  主账号 ".into()), ..key("  sk-test-a-long  ") };
    let first = store.add_key(&provider.id, labelled).unwrap();
    assert_eq!(first.revision, 2, "key operations bump the provider revision");
    let summary = &first.keys[0];
    assert_eq!((summary.label.as_deref(), summary.masked.as_str(), summary.enabled), (Some("主账号"), "sk****long", true));
    let error = store.add_key(&provider.id, key("sk-test-a-long")).unwrap_err();
    assert_eq!((error.kind(), error.to_string().contains("sk-test")), (Some(ErrorKind::Conflict), false));
    assert_eq!(store.reveal_key(&summary.id).unwrap().expose(), "sk-test-a-long", "trimmed original value");

    let second = store.add_key(&provider.id, key("sk-short")).unwrap();
    let short = second.keys[1].id.clone();
    assert_eq!(second.keys[1].masked, "••••••••");
    let duplicate = ProviderKeyUpdate { label: None, key: Some(Secret::new("sk-test-a-long")), enabled: true };
    kind(store.update_key(&provider.id, &short, duplicate), ErrorKind::Conflict);
    kind(store.update_key(&provider.id, &short, ProviderKeyUpdate { key: Some(Secret::new(" ")), ..toggle(true) }), ErrorKind::Invalid);

    let relabel = ProviderKeyUpdate { label: Some("备用".into()), key: None, enabled: false };
    let edited = store.update_key(&provider.id, &short, relabel).unwrap();
    assert_eq!((edited.revision, edited.keys[1].label.as_deref(), edited.keys[1].enabled), (4, Some("备用"), false));
    assert_eq!(store.reveal_key(&short).unwrap().expose(), "sk-short", "key None keeps the value");
    let replace = ProviderKeyUpdate { label: Some("备用".into()), key: Some(Secret::new("sk-test-replaced")), enabled: false };
    let replaced = store.update_key(&provider.id, &short, replace).unwrap();
    assert_eq!(replaced.keys[1].masked, "sk****aced");
    assert_eq!(store.reveal_key(&short).unwrap().expose(), "sk-test-replaced");

    let other = create(&store, input("other")).unwrap();
    kind(store.update_key(&other.id, &short, toggle(true)), ErrorKind::NotFound);
    kind(store.delete_key(&other.id, &short), ErrorKind::NotFound);
    let deleted = store.delete_key(&provider.id, &short).unwrap();
    assert_eq!((deleted.revision, deleted.keys.len()), (6, 1));
    kind(store.reveal_key(&short), ErrorKind::NotFound);
    assert!(!format!("{:?}", store.providers().unwrap()).contains("sk-test"), "listings carry no plaintext");
}

fn next(store: &SqliteSettingsStore, id: &ProviderId) -> String {
    store.next_key(id).unwrap().unwrap().value.expose().to_owned()
}

fn key_ids(provider: &Provider) -> Vec<ProviderKeyId> {
    provider.keys.iter().map(|key| key.id.clone()).collect()
}

#[test]
fn round_robin_rotation() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let provider = create(&store, input("relay")).unwrap();
    assert!(store.next_key(&provider.id).unwrap().is_none(), "no keys");
    kind(store.next_key(&ProviderId::from("missing")), ErrorKind::NotFound);
    for value in ["sk-test-a", "sk-test-b", "sk-test-c"] {
        store.add_key(&provider.id, key(value)).unwrap();
    }
    let ids = key_ids(&store.provider(&provider.id).unwrap());
    let order: Vec<_> = (0..4).map(|_| next(&store, &provider.id)).collect();
    assert_eq!(order, ["sk-test-a", "sk-test-b", "sk-test-c", "sk-test-a"], "rotates and wraps");

    store.providers().unwrap();
    store.provider(&provider.id).unwrap();
    store.reveal_key(&ids[2]).unwrap();
    assert_eq!(next(&store, &provider.id), "sk-test-b", "reads do not advance the rotation");

    store.update_key(&provider.id, &ids[1], toggle(false)).unwrap();
    let order: Vec<_> = (0..3).map(|_| next(&store, &provider.id)).collect();
    assert_eq!(order, ["sk-test-a", "sk-test-c", "sk-test-a"], "disabled middle key is skipped");

    store.update_key(&provider.id, &ids[2], toggle(false)).unwrap();
    let order: Vec<_> = (0..3).map(|_| next(&store, &provider.id)).collect();
    assert_eq!(order, ["sk-test-a"; 3], "a single enabled key is always returned");
    let selected = store.next_key(&provider.id).unwrap().unwrap();
    assert_eq!(selected.id, ids[0]);
    assert!(!format!("{selected:?}").contains("sk-test"));

    store.update_key(&provider.id, &ids[0], toggle(false)).unwrap();
    assert!(store.next_key(&provider.id).unwrap().is_none(), "all keys disabled");
}

#[test]
fn deleting_the_cursor_key_restarts_from_the_first() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let provider = create(&store, input("relay")).unwrap();
    for value in ["sk-test-a", "sk-test-b", "sk-test-c"] {
        store.add_key(&provider.id, key(value)).unwrap();
    }
    let ids = key_ids(&store.provider(&provider.id).unwrap());
    assert_eq!(next(&store, &provider.id), "sk-test-a");
    assert_eq!(next(&store, &provider.id), "sk-test-b");
    store.delete_key(&provider.id, &ids[1]).unwrap();
    assert_eq!(next(&store, &provider.id), "sk-test-a");
    assert_eq!(next(&store, &provider.id), "sk-test-c");

    let reopened = open(&directory);
    assert_eq!(next(&reopened, &provider.id), "sk-test-a", "the cursor is memory only");
}

fn stdio(name: &str, scope: McpScope) -> McpServerInput {
    McpServerInput {
        name: name.into(),
        scope,
        transport: McpTransport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "@example/mcp".into()],
            env: vec![EnvValue { name: "TOKEN".into(), value: "$EXAMPLE_TOKEN".into() }],
        },
        enabled: true,
        description: None,
    }
}

fn project(id: &str) -> McpScope {
    McpScope::Project { project: ProjectId::from(id) }
}

fn origins(store: &SqliteSettingsStore, project: &str) -> Vec<(String, McpOrigin, bool)> {
    store
        .effective(&ProjectId::from(project))
        .unwrap()
        .into_iter()
        .map(|entry| (entry.server.input.name, entry.origin, entry.enabled))
        .collect()
}

fn add(store: &SqliteSettingsStore, input: McpServerInput) -> McpServer {
    McpService::create(store, input).unwrap()
}

#[test]
fn mcp_names_and_transports_are_validated() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    for name in ["", "has space", "名字", "dot.name", &"n".repeat(33)] {
        kind(McpService::create(&store, stdio(name, McpScope::Global)), ErrorKind::Invalid);
    }
    let longest = add(&store, stdio(&"n".repeat(32), McpScope::Global));
    assert_eq!(longest.revision, 1);
    add(&store, stdio("Context7_mcp-1", McpScope::Global));
    let mut blank_command = stdio("blank", McpScope::Global);
    blank_command.transport = McpTransport::Stdio { command: "  ".into(), args: Vec::new(), env: Vec::new() };
    kind(McpService::create(&store, blank_command), ErrorKind::Invalid);
    let http = |url: &str, header: &str| McpServerInput {
        transport: McpTransport::Http {
            url: url.into(),
            headers: vec![EnvValue { name: header.into(), value: "${MCP_TOKEN}".into() }],
        },
        ..stdio("http", McpScope::Global)
    };
    kind(McpService::create(&store, http("ws://mcp.example", "Authorization")), ErrorKind::Invalid);
    kind(McpService::create(&store, http("https://u:p@mcp.example/mcp", "Authorization")), ErrorKind::Invalid);
    kind(McpService::create(&store, http("https://mcp.example/mcp", " ")), ErrorKind::Invalid);
    kind(McpService::create(&store, stdio("p", project(" "))), ErrorKind::Invalid);
    let created = add(&store, http(" https://mcp.example/mcp ", "Authorization"));
    assert_eq!(created.input, http("https://mcp.example/mcp", "Authorization"), "url trimmed");

    kind(McpService::create(&store, stdio("http", McpScope::Global)), ErrorKind::Conflict);
    add(&store, stdio("http", project("p1")));
    add(&store, stdio("http", project("p2")));
    kind(McpService::create(&store, stdio("http", project("p1"))), ErrorKind::Conflict);
    assert_eq!(store.global().unwrap().len(), 3);
}

#[test]
fn project_entries_replace_same_named_globals_and_overrides_toggle() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let context7 = add(&store, stdio("context7", McpScope::Global));
    let deepwiki = add(&store, stdio("deepwiki", McpScope::Global));
    let mut disabled = stdio("offline", McpScope::Global);
    disabled.enabled = false;
    let offline = add(&store, disabled);
    let local = add(&store, stdio("local", project("p1")));
    let expected_p1 = vec![
        ("context7".into(), McpOrigin::Global, true),
        ("deepwiki".into(), McpOrigin::Global, true),
        ("offline".into(), McpOrigin::Global, false),
        ("local".into(), McpOrigin::Project, true),
    ];
    assert_eq!(origins(&store, "p1"), expected_p1);

    store.set_override(&ProjectId::from("p1"), &context7.id, Some(false)).unwrap();
    store.set_override(&ProjectId::from("p1"), &offline.id, Some(true)).unwrap();
    let p1 = origins(&store, "p1");
    assert_eq!((p1[0].2, p1[2].2), (false, true), "project overrides both ways");
    assert!(store.global().unwrap()[0].input.enabled, "global entry itself unchanged");
    assert_eq!(origins(&store, "p2")[0], ("context7".into(), McpOrigin::Global, true), "other projects unaffected");
    store.set_override(&ProjectId::from("p1"), &context7.id, None).unwrap();
    store.set_override(&ProjectId::from("p1"), &offline.id, None).unwrap();
    assert_eq!(origins(&store, "p1"), expected_p1, "None clears the override");

    kind(store.set_override(&ProjectId::from("p1"), &local.id, Some(false)), ErrorKind::Invalid);
    kind(store.set_override(&ProjectId::from("p1"), &"missing".into(), Some(false)), ErrorKind::NotFound);

    store.set_override(&ProjectId::from("p1"), &deepwiki.id, Some(true)).unwrap();
    let replacement = add(&store, stdio("deepwiki", project("p1")));
    let p1 = store.effective(&ProjectId::from("p1")).unwrap();
    assert_eq!((&p1[1].server.id, p1[1].origin, p1[1].enabled), (&deepwiki.id, McpOrigin::GlobalOverridden, false));
    assert_eq!((&p1[4].server.id, p1[4].origin), (&replacement.id, McpOrigin::Project));
    assert_eq!(origins(&store, "p2")[1], ("deepwiki".into(), McpOrigin::Global, true));
}

#[test]
fn mcp_update_delete_and_stable_effective_order() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let first = add(&store, stdio("zeta", McpScope::Global));
    add(&store, stdio("alpha", McpScope::Global));
    add(&store, stdio("local-b", project("p1")));
    add(&store, stdio("local-a", project("p1")));
    let names = |store: &SqliteSettingsStore| -> Vec<String> {
        origins(store, "p1").into_iter().map(|(name, ..)| name).collect()
    };
    let order = names(&store);
    assert_eq!(order, ["zeta", "alpha", "local-b", "local-a"], "globals first, each in creation order");
    assert_eq!(names(&store), order);

    let mut edited = stdio("zeta", McpScope::Global);
    edited.description = Some("  说明  ".into());
    edited.enabled = false;
    let updated = McpService::update(&store, &first.id, first.revision, edited).unwrap();
    assert_eq!((updated.revision, updated.input.description.as_deref(), updated.input.enabled), (2, Some("说明"), false));
    assert_eq!(names(&open(&directory)), order, "editing and reopening keep the order");
    kind(McpService::update(&store, &first.id, first.revision, stdio("zeta", McpScope::Global)), ErrorKind::Conflict);
    kind(McpService::update(&store, &first.id, updated.revision, stdio("alpha", McpScope::Global)), ErrorKind::Conflict);

    store.set_override(&ProjectId::from("p1"), &first.id, Some(true)).unwrap();
    kind(McpService::delete(&store, &first.id, first.revision), ErrorKind::Conflict);
    McpService::delete(&store, &first.id, updated.revision).unwrap();
    assert_eq!(count(&store, "SELECT COUNT(*) FROM mcp_overrides WHERE server_id = ?1", first.id.as_str()), 0);
    assert_eq!(names(&store), ["alpha", "local-b", "local-a"]);
    kind(McpService::delete(&store, &first.id, updated.revision), ErrorKind::NotFound);
}

#[test]
fn model_prefs_defaults_and_hidden_models() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    assert_eq!(store.prefs().unwrap(), Default::default());
    let provider = create(&store, input("relay")).unwrap();
    let gpt = ModelRef { provider: provider.id.clone(), model_id: "gpt-test".into() };
    let claude = ModelRef { provider: provider.id.clone(), model_id: "claude-test".into() };
    let missing = ModelRef { provider: provider.id.clone(), model_id: "missing".into() };

    let prefs = store.set_default(Some(claude.clone()), Some(ThinkingLevel::High)).unwrap();
    assert_eq!((prefs.default_model.as_ref(), prefs.default_effort), (Some(&claude), Some(ThinkingLevel::High)));
    assert_eq!(open(&directory).prefs().unwrap(), prefs, "defaults are durable");
    let prefs = store.set_default(Some(gpt.clone()), None).unwrap();
    assert_eq!((prefs.default_model.as_ref(), prefs.default_effort), (Some(&gpt), None));
    kind(store.set_default(Some(missing.clone()), None), ErrorKind::NotFound);
    assert_eq!(store.prefs().unwrap(), prefs, "a rejected default changes nothing");

    store.set_hidden(&gpt, true).unwrap();
    let prefs = store.set_hidden(&claude, true).unwrap();
    assert_eq!(prefs.hidden, vec![claude.clone(), gpt.clone()]);
    assert_eq!(store.set_hidden(&gpt, true).unwrap().hidden.len(), 2, "hiding twice is idempotent");
    assert_eq!(store.set_hidden(&gpt, false).unwrap().hidden, vec![claude.clone()]);
    kind(store.set_hidden(&missing, true), ErrorKind::NotFound);

    let mut without_claude = input("relay");
    without_claude.models.retain(|model| model.model_id == "gpt-test");
    let provider = store.provider(&provider.id).unwrap();
    update(&store, &provider, without_claude).unwrap();
    let prefs = store.prefs().unwrap();
    assert!(prefs.hidden.is_empty(), "deleting a model removes its hidden record");
    assert_eq!(prefs.default_model, Some(gpt.clone()));

    let provider = store.provider(&provider.id).unwrap();
    ProviderService::delete(&store, &provider.id, provider.revision).unwrap();
    assert_eq!(store.prefs().unwrap().default_model, Some(gpt), "a dangling default is kept for the UI");
    assert_eq!(store.set_default(None, None).unwrap(), Default::default());
}

fn snapshot(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name().to_string_lossy().into_owned(), fs::read(entry.path()).unwrap())
        })
        .collect();
    files.sort();
    files
}

#[test]
fn migration_creates_tables_and_reopens_durably() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    let tables = store
        .read(|conn| {
            conn.rows("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name", ())?
                .iter()
                .map(|row| crate::db::text(row, 0))
                .collect::<Result<Vec<_>>>()
        })
        .unwrap();
    assert_eq!(
        tables,
        [
            "hidden_models",
            "mcp_overrides",
            "mcp_servers",
            "model_prefs",
            "provider_endpoints",
            "provider_keys",
            "provider_models",
            "providers",
            "schema_meta"
        ]
    );
    assert_eq!(store.read(|conn| conn.int("SELECT version FROM schema_meta", ())).unwrap(), 1);
    let provider = create(&store, input("relay")).unwrap();
    let provider = store.add_key(&provider.id, key("sk-test-a")).unwrap();
    let server = add(&store, stdio("context7", McpScope::Global));
    drop(store);

    let reopened = open(&directory);
    assert_eq!(reopened.providers().unwrap(), vec![provider.clone()]);
    assert_eq!(reopened.reveal_key(&provider.keys[0].id).unwrap().expose(), "sk-test-a");
    assert_eq!(reopened.global().unwrap(), vec![server]);
    assert_eq!(reopened.read(|conn| conn.int("SELECT COUNT(*) FROM schema_meta", ())).unwrap(), 1);
    let header = fs::read(directory.path().join("data").join(DB)).unwrap();
    assert!(header.starts_with(b"SQLite format 3\0"));
}

#[test]
fn newer_schema_is_corrupt_and_left_untouched() {
    let directory = TempDir::new().unwrap();
    let store = open(&directory);
    create(&store, input("relay")).unwrap();
    store.write(|conn| conn.execute("UPDATE schema_meta SET version = 999", ())).unwrap();
    drop(store);
    let data = directory.path().join("data");
    let before = snapshot(&data);
    let error = SqliteSettingsStore::open(&data.join(DB)).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Corrupt), "{error}");
    assert!(error.to_string().contains("更新版本"), "{error}");
    assert_eq!(snapshot(&data), before, "file bytes unchanged");
}

#[test]
fn unwritable_location_is_invalid() {
    let directory = TempDir::new().unwrap();
    let blocker = directory.path().join("data");
    fs::write(&blocker, b"not a directory").unwrap();
    let error = SqliteSettingsStore::open(&blocker.join(DB)).unwrap_err();
    assert_eq!(error.kind(), Some(ErrorKind::Invalid), "{error}");
    assert!(error.to_string().contains("--data-dir"), "{error}");
}
