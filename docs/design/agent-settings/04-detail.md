# 04 详细设计：模型、提供方、MCP 服务器

结构见 [03 概要设计](03-architecture.md)；任务见 [05](05-tasks.md)。代码风格、错误、日志、持久化、测试约定全部沿用多 Agent 设计包 [04](../native-multi-agent/04-detail.md) §1–§3、§7、§11，这里不重复，只写本特性的契约。所有 cargo 命令经 `scripts\msvc\cargo-msvc.bat` 执行。

## 1. 约定要点

- 服务方法都是**同步阻塞**的，由 `app::state::run` 放到后台执行器调用。异步库（Turso、tokio）只在实现内部使用。
- 库层错误只用 `thinkrail_core::Error`。新增的错误不增加 `ErrorKind`，按语义复用：校验失败 `Invalid`；重复 `Conflict`；revision 过期 `Conflict`；找不到 `NotFound`；数据库损坏或版本过新 `Corrupt`；上游拒绝或不可达 `External`；超时 `Timeout`；上游返回非预期格式 `Protocol`；网关未启动 `BackendUnavailable`。消息用中文，写明哪里出错、怎么办，**不得包含密钥、请求头值、环境变量值**。
- 日志字段：`provider`（id）、`route`、`key`（密钥 id 或标签，不是值）、`status`、`elapsed_ms`、`code`。
- 测试位置按模块类型区分：core/service 的纯服务测试可使用 owning module 的 `tests.rs`；app 的 feature-level UI/integration contract tests 必须放在 `crates/app/tests/` 或 dedicated test crate，不能注册到生产 `src/` feature module。测试使用临时目录、占位密钥（如 `sk-test-a`）和本机回环上的假上游；不接触真实凭据。需要真实引擎或真实中转的测试标 `#[ignore = "needs …"]`。

## 2. core 契约

### 2.1 新模块与 id

`crates/core/src/lib.rs` 新增模块 `provider`、`mcp`、`routing`，并 `pub use` 导出。`ids.rs` 的 `id!` 宏新增 `ProviderId`、`ProviderKeyId`、`McpServerId`。

### 2.2 提供方（`provider.rs`）

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Protocol { OpenaiChat, OpenaiResponses, AnthropicMessages } // "openai-chat" / "openai-responses" / "anthropic-messages"

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access { Direct, Gateway }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel { Off, Minimal, Low, Medium, High, Xhigh, Max }

pub struct Endpoint { pub protocol: Protocol, pub base_url: String }
pub struct Header { pub name: String, pub value: String }

pub struct ProviderModel {
    pub model_id: String, pub name: String,
    pub protocols: Vec<Protocol>,          // 有序，非空；第一个是默认
    pub context_window: Option<u32>, pub max_tokens: Option<u32>,
    pub vision: bool, pub reasoning: bool, pub tools: bool,
    pub efforts: Vec<ThinkingLevel>,       // 推理模型声明的档位；不推理时为空
    pub enabled: bool,
}

pub struct ProviderInput {
    pub name: String, pub endpoints: Vec<Endpoint>, pub default_protocol: Protocol,
    pub headers: Vec<Header>, pub access: Access, pub enabled: bool,
    pub models: Vec<ProviderModel>,
}

pub struct ProviderKeySummary { pub id: ProviderKeyId, pub label: Option<String>, pub masked: String, pub enabled: bool }
pub struct Provider { pub id: ProviderId, pub revision: u64, pub input: ProviderInput, pub keys: Vec<ProviderKeySummary> }

pub struct ProviderKeyInput { pub label: Option<String>, pub key: Secret, pub enabled: bool }
/// 修改密钥：`key` 为 None 时只改标签和启用状态。
pub struct ProviderKeyUpdate { pub label: Option<String>, pub key: Option<Secret>, pub enabled: bool }
/// 选中的密钥；值只在宿主进程内使用。
pub struct SelectedKey { pub id: ProviderKeyId, pub label: Option<String>, pub value: Secret }

/// 密钥明文。`Debug` 输出 `Secret(***)`；不实现 `Display`、`Serialize`；`expose` 是唯一取值方式。
pub struct Secret(String);
impl Secret { pub fn new(value: impl Into<String>) -> Self; pub fn expose(&self) -> &str; }
```

- 除 `Secret`、`ProviderKeyInput`、`ProviderKeyUpdate`、`SelectedKey` 外，以上类型都 `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]`，`rename_all = "camelCase"`。这四个只 `derive(Clone)`，`Debug` 手写并隐去值。
- `masked` 由存储层生成，规则同 Cherry `maskApiKey`：长度大于 8 时为前 2 位 + `****` + 后 4 位，否则 `••••••••`。

```rust
pub trait ProviderService: Send + Sync {
    fn providers(&self) -> Result<Vec<Provider>>;              // 按 sort 排序；不含明文
    fn provider(&self, id: &ProviderId) -> Result<Provider>;
    fn create(&self, input: ProviderInput) -> Result<Provider>;
    fn update(&self, id: &ProviderId, expect_revision: u64, input: ProviderInput) -> Result<Provider>;
    fn delete(&self, id: &ProviderId, expect_revision: u64) -> Result<()>; // 级联删除端点、模型、密钥
    fn add_key(&self, id: &ProviderId, key: ProviderKeyInput) -> Result<Provider>;
    fn update_key(&self, id: &ProviderId, key_id: &ProviderKeyId, key: ProviderKeyUpdate) -> Result<Provider>;
    fn delete_key(&self, id: &ProviderId, key_id: &ProviderKeyId) -> Result<Provider>;
    /// 显示 / 复制：读取某个密钥的明文；不推进轮换。
    fn reveal_key(&self, key_id: &ProviderKeyId) -> Result<Secret>;
    /// 轮询选出下一个已启用的密钥（02 §3.4）；会推进轮换。没有已启用密钥时返回 None。
    fn next_key(&self, id: &ProviderId) -> Result<Option<SelectedKey>>;
}
```

- 密钥操作都让提供方 `revision` 加 1，并返回更新后的 `Provider`。
- 提供方**没有任何密钥**时视为不需要鉴权（本机模型服务），投影时用占位值 `thinkrail-no-key`（与原版一致）；**有密钥但全部停用**时，`ModelRouter::prepare` 报 `Invalid`：“提供方 {name} 没有启用的 API 密钥，请在设置 → 提供方中启用一个。”

### 2.3 MCP（`mcp.rs`）

```rust
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum McpScope { Global, Project { project: ProjectId } }

pub struct EnvValue { pub name: String, pub value: String } // value 可为明文，或 `$VAR` / `${VAR}` 引用宿主启动环境

#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpTransport {
    Stdio { command: String, args: Vec<String>, env: Vec<EnvValue> },
    Http { url: String, headers: Vec<EnvValue> },
}

pub struct McpServerInput { pub name: String, pub scope: McpScope, pub transport: McpTransport, pub enabled: bool, pub description: Option<String> }
pub struct McpServer { pub id: McpServerId, pub revision: u64, pub input: McpServerInput }

/// 某个项目里实际生效的一项；`origin` 用于界面显示“来自全局”。
pub struct EffectiveMcpServer { pub server: McpServer, pub origin: McpOrigin, pub enabled: bool }
pub enum McpOrigin { Global, Project, GlobalOverridden } // GlobalOverridden：被同名项目级条目替换，enabled 恒为 false

pub trait McpService: Send + Sync {
    fn global(&self) -> Result<Vec<McpServer>>;
    /// 全局 + 该项目的条目，已应用项目覆盖和同名替换（02 §3.2）。
    fn effective(&self, project: &ProjectId) -> Result<Vec<EffectiveMcpServer>>;
    fn create(&self, input: McpServerInput) -> Result<McpServer>;
    fn update(&self, id: &McpServerId, expect_revision: u64, input: McpServerInput) -> Result<McpServer>;
    fn delete(&self, id: &McpServerId, expect_revision: u64) -> Result<()>;
    /// 项目对全局条目的启停；None 清除覆盖、回到全局设置。
    fn set_override(&self, project: &ProjectId, server: &McpServerId, enabled: Option<bool>) -> Result<()>;
}
```

校验：名称 `^[A-Za-z0-9_-]{1,32}$`，同一范围内唯一（全局之间、同一项目之间）；stdio 命令非空；HTTP 地址只能是 `http(s)`，不能带用户名密码；`EnvValue.name` 非空；`set_override` 只接受全局条目。

### 2.4 模型偏好（`provider.rs`）

```rust
pub struct ModelRef { pub provider: ProviderId, pub model_id: String }
pub struct ModelPrefs { pub default_model: Option<ModelRef>, pub default_effort: Option<ThinkingLevel>, pub hidden: Vec<ModelRef> }

pub trait ModelPrefService: Send + Sync {
    fn prefs(&self) -> Result<ModelPrefs>;
    fn set_default(&self, model: Option<ModelRef>, effort: Option<ThinkingLevel>) -> Result<ModelPrefs>;
    fn set_hidden(&self, model: &ModelRef, hidden: bool) -> Result<ModelPrefs>;
}
```

只记录被隐藏的模型，新模型默认可见（与原版一致）。删除提供方或模型时，同时删除指向它的隐藏记录；默认模型指向已删除的模型时保留记录，由界面显示“默认模型不可用”。

### 2.5 路由（`routing.rs`）

```rust
pub struct RouteRequest { pub chat: ChatId, pub project: ProjectId, pub backend: BackendKind, pub new_session: bool }

/// 某次引擎启动要投影的全部内容。没有任何提供方和 MCP 时各字段为空，引擎照旧使用自己的配置。
/// 所有敏感值（密钥、网关令牌、请求头值、MCP 的 env / headers 值）只在 `env` 里；其余字段只引用变量名，
/// 适配器把它们写进生成文件也不会泄露。
pub struct EngineRouting {
    pub providers: Vec<ProjectedProvider>,
    pub default_model: Option<DefaultModel>,   // 只在 new_session 时给出
    pub mcp: Vec<ProjectedMcpServer>,
    pub env: Vec<(String, Secret)>,            // (变量名, 值)，追加到引擎子进程环境
    pub signature: String,                     // 见 §4.4
}
/// (名称, 宿主注入的环境变量名)。名称是请求头名或 MCP 服务器内的变量名。
pub struct EnvBinding { pub name: String, pub var: String }
pub struct ProjectedProvider {
    pub route: String,          // `thinkrail-<提供方 id 前 8 位>-<oc|or|am>`，引擎内的提供方名
    pub display_name: String, pub protocol: Protocol,
    pub base_url: String,       // 已按引擎规则规范化（§4.2）；网关方式下指向网关
    pub key_var: String,        // `THINKRAIL_PROVIDER_<短 id>_KEY`
    pub headers: Vec<EnvBinding>, // 变量名 `THINKRAIL_HDR_<短 id>_<序号>`；网关方式下为空（由网关添加）
    pub models: Vec<ProjectedModel>,
}
pub struct ProjectedModel { pub id: String, pub name: String, pub vision: bool, pub reasoning: bool, pub efforts: Vec<ThinkingLevel>, pub context_window: u32, pub max_tokens: u32 }
pub struct DefaultModel { pub route: String, pub model_id: String, pub effort: Option<ThinkingLevel> }
pub struct ProjectedMcpServer { pub name: String /* `thinkrail-<名称>` */, pub transport: ProjectedMcpTransport }
/// 值已由宿主解析（`$VAR` 换成宿主启动环境里的值）后放进 `EngineRouting.env`，变量名 `THINKRAIL_MCP_<服务器 id 前 8 位>_<E|H><序号>`。
pub enum ProjectedMcpTransport {
    Stdio { command: String, args: Vec<String>, env: Vec<EnvBinding> },
    Http { url: String, headers: Vec<EnvBinding> },
}
pub struct ModelChoice { pub provider: ProviderId, pub provider_name: String, pub route: String, pub model: ProjectedModel }

pub trait ModelRouter: Send + Sync {
    /// 生成一次启动的路由。直连方式推进密钥轮换；网关方式为该聊天登记新会话令牌（§5.2）。
    fn prepare(&self, request: &RouteRequest) -> Result<EngineRouting>;
    /// 与 prepare 相同输入下的签名；只读，不推进轮换、不登记令牌。
    fn signature(&self, request: &RouteRequest) -> Result<String>;
    /// 引擎退出或聊天关闭时调用，作废该聊天的网关令牌。
    fn release(&self, chat: &ChatId);
    /// 该引擎可选的模型：启用的提供方 × 启用且未隐藏的模型 × 引擎支持的协议。
    fn catalog(&self, backend: BackendKind) -> Result<Vec<ModelChoice>>;
}

pub struct ProbeTarget { pub endpoint: Endpoint, pub headers: Vec<Header>, pub key: Option<Secret> }
pub struct DiscoveredModel { pub id: String, pub name: Option<String>, pub context_window: Option<u32> }
pub struct KeyCheck { pub key: ProviderKeyId, pub ok: bool, pub elapsed_ms: u64, pub error: Option<String> }

pub trait ProviderProbe: Send + Sync {
    /// “查找模型”：请求上游模型列表（§5.3）。
    fn list_models(&self, target: &ProbeTarget) -> Result<Vec<DiscoveredModel>>;
    /// “检测”：用指定模型发一次最小请求，15 秒超时、不重试；返回耗时毫秒。
    fn check(&self, target: &ProbeTarget, model_id: &str) -> Result<u64>;
    /// “检测所有密钥”：对每个已启用密钥各检测一次；用显式密钥，不推进轮换。
    fn check_keys(&self, provider: &ProviderId, model_id: &str) -> Result<Vec<KeyCheck>>;
}
```

`KeyCheck.error` 是已脱敏的中文说明：状态码 + 上游 `error.message` 的前 200 字符，经 `redact` 处理。

- §2.3、§2.5 的类型同样 `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]`、`rename_all = "camelCase"`（`McpOrigin` 另加 `Copy`）。含 `Secret` 的 `EngineRouting`、`ProbeTarget` 只 `derive(Clone)`（`EngineRouting` 另加 `Default`），`Debug` 手写：`Secret` 显示为 `Secret(***)`，`ProbeTarget.headers` 只显示请求头名。

### 2.6 引擎契约的改动（`agent.rs`、`host_tools.rs`）

```rust
pub struct StartSession { /* 现有字段 */ pub routing: EngineRouting, pub support_dir: PathBuf }
pub struct Capabilities { /* 现有字段 */ pub model_switch: bool }   // 三个引擎都为 true
pub struct ModelInfo { pub provider: Option<String>, pub id: String, pub effort: Option<ThinkingLevel> }

pub trait AgentSession: Send + Sync {
    /* 现有方法 */
    /// 只对当前会话生效，不写回引擎配置。`provider` 是 `ProjectedProvider.route`。
    fn set_model(&self, provider: &str, model_id: &str, effort: Option<ThinkingLevel>) -> Result<()>;
}
```

- `support_dir` 是本次启动的引擎辅助目录 `<data>/engines/<聊天 id>/`，由 app 在启动前创建、聊天关闭时删除。`HostToolAccess.support_dir` 删掉，宿主工具的扩展文件也写进这里，每个聊天的生成文件互不覆盖。
- `EngineRouting` 为空时（没有提供方、没有 MCP），三个适配器的行为必须与现在完全一致：不加 `-e`、`--config`、patch 条目，不注入环境变量（回归要求）。

## 3. settings-store crate

### 3.1 结构

```
crates/settings-store/
  Cargo.toml        # thinkrail-core, turso =0.8.3（default-features = false）, pollster =1.0.1, parking_lot, serde, serde_json（JSON 列）, tracing; dev: tempfile
  src/lib.rs        # pub use store::SqliteSettingsStore;
  src/store.rs      # SqliteSettingsStore 结构与 open
  src/db.rs         # 打开数据库、每个连接执行 PRAGMA、迁移、事务辅助、行解码
  src/schema.rs     # SCHEMA_VERSION 与建表 SQL
  src/providers.rs  # impl ProviderService
  src/mcp.rs        # impl McpService
  src/prefs.rs      # impl ModelPrefService
  src/validate.rs   # 输入校验（中文消息）
  src/mask.rs       # maskApiKey
  src/tests.rs
```

`SqliteSettingsStore` 同时实现三个 trait。字段：`db: turso::Database`、`writer: parking_lot::Mutex<()>`（进程内串行化写事务）、`cursor: parking_lot::Mutex<HashMap<ProviderId, ProviderKeyId>>`（轮询游标）。

- `pub fn open(path: &Path) -> Result<Self>`：创建父目录；`Builder::new_local(path).build()`；执行迁移；路径不可写时报 `Invalid`：“无法在 {目录} 创建数据库，请用 --data-dir 指定可写目录。”
- 每个方法用 `pollster::block_on` 驱动异步调用，不引入 tokio（M0 实测可行）。
- 每次 `db.connect()` 后立即执行 `PRAGMA foreign_keys = ON`（M0：这是每个连接的设置）。
- 写操作：拿 `writer` 锁 → `BEGIN IMMEDIATE` → 读取并校验 revision → 写 → `COMMIT`；任一步失败 `ROLLBACK`。多语句读取在 `BEGIN … COMMIT` 里执行，得到一致的快照。
- 线程共享（T2 实测，2026-10-31，turso 0.8.3）：`turso::Database` 是 `Send + Sync`（内部是 `Arc`，`Connection` 也有 `assert_send_sync`）；两个线程共享同一个 `Database`、各自 `connect()` 并发写入，100 行全部落盘（测试 `turso_database_is_shared_across_threads`）；4 个线程并发调用 `SqliteSettingsStore` 的新建提供方、添加密钥、列表也全部成功（`store_writes_from_several_threads`）。所以 `open` 只构建一次 `Database`，每次方法调用 `connect()` 一个新连接，并设 `busy_timeout` 5 秒。
- 约束错误映射：`UNIQUE constraint failed` → `Conflict`（如“已有同名提供方”）；其余数据库错误 → `Corrupt`（打开或迁移时）或 `External`（运行时），消息不含 SQL 参数。

### 3.2 表结构（`schema.rs`，SCHEMA_VERSION = 3；v2 打开时原地加 `provider_models.model_type` 列并按 id 推断类型）

```sql
CREATE TABLE schema_meta (version INTEGER NOT NULL);
CREATE TABLE providers (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, default_protocol TEXT NOT NULL,
  headers TEXT NOT NULL,            -- JSON [{name,value}]
  access TEXT NOT NULL, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
  revision INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE provider_endpoints (
  provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  protocol TEXT NOT NULL, base_url TEXT NOT NULL, PRIMARY KEY (provider_id, protocol));
CREATE TABLE provider_models (
  provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  model_id TEXT NOT NULL, name TEXT NOT NULL, protocols TEXT NOT NULL,   -- JSON ["anthropic-messages", ...]
  context_window INTEGER, max_tokens INTEGER, vision INTEGER NOT NULL, reasoning INTEGER NOT NULL,
  tools INTEGER NOT NULL, efforts TEXT NOT NULL, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
  PRIMARY KEY (provider_id, model_id));
CREATE TABLE provider_keys (
  id TEXT PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  key TEXT NOT NULL, label TEXT, enabled INTEGER NOT NULL, sort INTEGER NOT NULL,
  UNIQUE (provider_id, key));
CREATE TABLE model_prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- default_model / default_effort（JSON）
CREATE TABLE hidden_models (
  provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE, model_id TEXT NOT NULL,
  PRIMARY KEY (provider_id, model_id));
CREATE TABLE mcp_servers (
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  project_id TEXT NOT NULL,         -- 全局为 ''，避免 NULL 的唯一性语义
  transport TEXT NOT NULL,          -- JSON McpTransport
  enabled INTEGER NOT NULL, description TEXT, revision INTEGER NOT NULL, sort INTEGER NOT NULL,
  UNIQUE (project_id, name));
CREATE TABLE mcp_overrides (
  project_id TEXT NOT NULL, server_id TEXT NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
  enabled INTEGER NOT NULL, PRIMARY KEY (project_id, server_id));
```

- 迁移：库为空时建全部表并写 `schema_meta`；版本等于当前版本时直接用；版本更高时报 `Corrupt`：“数据库由更新版本的 ThinkRail 创建，请升级后再打开。”，不修改文件。
- `update(provider)` 在一个事务里：更新 `providers` 行、删除并重写该提供方的 `provider_endpoints`、`provider_models`；保留 `provider_keys`；删除不再存在的模型对应的 `hidden_models` 行。
- 排序：新建项的 `sort` 取当前最大值加 1；列表按 `sort, name` 返回。

### 3.3 校验（`validate.rs`）

| 字段 | 规则 | 错误（`Invalid`） |
|---|---|---|
| 提供方名称 | 去首尾空白后 1–64 字符，无控制字符 | “请填写提供方名称（最多 64 个字符）。” |
| 端点 | 至少一个；`protocol` 不重复；地址去空白后必须以 `http://` 或 `https://` 开头，不含用户名密码；末尾 `#` 允许 | “API 地址需要以 http:// 或 https:// 开头。” |
| 默认协议 | 必须是已配置地址的协议之一 | “默认协议没有对应的 API 地址。” |
| 请求头 | 名称是合法 HTTP 头名，不能是 `authorization`、`x-api-key`、`host`、`content-length` | “请求头 {name} 由 ThinkRail 管理，不能自定义。” |
| 模型 | 至多 500 个；`model_id` 非空且在提供方内唯一；`protocols` 非空且每项都配置了地址；`reasoning=false` 时 `efforts` 为空 | “模型 {id} 的端点类型没有对应的 API 地址。” |
| 密钥 | 去首尾空白后非空；同一提供方内不重复 | “API 密钥不能为空” / “API 密钥已存在”（`Conflict`） |
| 标签 | 可空，最多 64 字符 | — |
| MCP | 见 §2.3 | — |

### 3.4 密钥轮询（`providers.rs::next_key`）

照搬 Cherry `resolveApiKey`（01 §4.5）：

1. 读该提供方 `enabled = 1` 的密钥，按 `sort` 排序；0 个返回 `None`，1 个直接返回。
2. 多个时读 `cursor[provider]`：没有记录或记录的 id 不在列表里，返回第一个；否则返回它后面的一个（到末尾回到第一个）。
3. 把返回的 id 写回 `cursor`。游标只在内存，不落盘。

`reveal_key`、`providers`、`provider` 都不读写游标。

## 4. routing crate：路由解析

### 4.1 结构

```
crates/routing/
  Cargo.toml       # thinkrail-core, tokio =1.54.1（rt-multi-thread, net, macros, time）, hyper =1.12.0（server, http1）,
                   # hyper-util =0.1.21（server-auto, tokio）, http-body-util =0.1.5, bytes =1.12.1, futures-util =0.3.34,
                   # reqwest =0.13.5（default-features = false: rustls-no-provider, stream, http2, json）,
                   # rustls =0.23.46（default-features = false: ring, std, tls12, logging）, sha2, serde_json, parking_lot, uuid, tracing
  src/lib.rs       # pub use {router::Router, gateway::Gateway, probe::Probe};
  src/router.rs    # impl ModelRouter
  src/url.rs       # 地址规范化（§4.2）
  src/signature.rs # 签名（§4.4）
  src/gateway.rs   # 回环网关（§5）
  src/probe.rs     # impl ProviderProbe（§5.3）
  src/runtime.rs   # 进程内唯一的 tokio 多线程运行时，安装 rustls ring 提供方
  src/tests.rs
```

`Router::new(providers: Arc<dyn ProviderService>, mcp: Arc<dyn McpService>, prefs: Arc<dyn ModelPrefService>, gateway: Arc<dyn GatewayRoutes>)`。`GatewayRoutes` 是 routing crate 内部的窄接口，`Gateway`（§5.2）实现它，路由解析的测试可以换成记录调用的替身：

```rust
pub trait GatewayRoutes: Send + Sync {
    fn base_url(&self, route: &str, protocol: Protocol) -> Result<String>;
    fn register(&self, chat: &ChatId) -> Secret;
    fn release(&self, chat: &ChatId);
}
```

### 4.2 地址规范化（`url.rs`，照搬 Cherry `formatApiHost` / `withoutTrailingApiVersion`）

```rust
pub(crate) fn base_url(raw: &str, protocol: Protocol) -> String
```

1. 去首尾空白，去末尾 `/`。
2. 末尾是 `#`：去掉 `#` 原样返回。
3. `OpenaiChat` / `OpenaiResponses`：路径里已有 `/v<数字>`（可带 `alpha`/`beta`）就不动，否则补 `/v1`。
4. `AnthropicMessages`：去掉末尾的 `/v<数字>…`；三个引擎都用 Anthropic SDK，会自己拼 `/v1/messages`。

预览复用规范化后的 base 并附加 SDK 实际资源路径；`#` 只禁止自动规范化 base，不会让 SDK 停止附加 `/chat/completions`、`/responses` 或 `/v1/messages`。

单元测试覆盖：`https://a.com`、`https://a.com/`、`https://a.com/v1`、`https://a.com/v1/`、`https://a.com/api/v3`、`https://a.com/v1#`、`https://a.com/anthropic`（DeepSeek 风格）× 三种协议。

### 4.3 `prepare` 算法

```text
输入 RouteRequest{chat, project, backend, new_session}
1. providers = ProviderService::providers() 中 enabled 的；models 取 enabled 的。
2. 对每个提供方 p、每个协议 proto ∈ p.endpoints：
     若 backend 不支持 proto（§4.5）→ 跳过；
     models_for(proto) = p.models 中 protocols 含 proto 的；
     为空 → 跳过。
   每个 (p, proto) 生成一个 ProjectedProvider：
     route       = "thinkrail-" + p.id[0..8] + "-" + {oc|or|am}
     key_var     = "THINKRAIL_PROVIDER_" + upper(p.id[0..8]) + "_KEY"
     direct:  base_url = url::base_url(endpoint, proto)；
              headers：第 i 个请求头 → EnvBinding{name, var: "THINKRAIL_HDR_" + upper(p.id[0..8]) + "_" + i}，值进 env；
              密钥：p 没有任何密钥 → "thinkrail-no-key"；否则 next_key(p) → None 时报 Invalid（§2.2）。
              同一提供方的多个协议共用一次 next_key 的结果（一次启动只轮换一次）。
              backend 是 DSH 且 p 有额外请求头 → 这个提供方改按 gateway 处理（§6.4）。
     gateway: base_url = Gateway::base_url(route, proto)；headers = []；密钥 = 该聊天的会话令牌（同一聊天所有路由共用一个）。
   env 写入 (key_var, 密钥或令牌) 以及所有请求头值。
3. 模型：每个 ProjectedModel 的 context_window 缺省 128000，max_tokens 缺省 16384（pi 的默认值）；efforts 只在 reasoning 时保留。
4. default_model（只在 new_session）：
     prefs.default_model 指向的模型在某个 ProjectedProvider 里 → DefaultModel{route, model_id, effort}；
       模型声明了多个协议时，按 protocols 顺序选第一个该引擎支持的；
     effort = prefs.default_effort，若不在模型 efforts 里则取 efforts 中不超过它的最大档；模型不推理时为 None；
     没有默认模型或默认模型不可用 → None（引擎用自己的默认），并在日志 info 一次。
5. mcp = McpService::effective(project) 中 enabled 的条目，第 n 个：
     name = "thinkrail-" + 名称；
     env / headers 的第 i 项 → EnvBinding{name, var: "THINKRAIL_MCP_" + upper(server.id[0..8]) + "_" + {E|H} + i}，值进 env；
     变量名只由服务器 id 和位置决定，不依赖本次生效集合里的顺序，所以多个聊天共用 OMP 的 mcp.json 时引用不会错位。
     值是 `$VAR` 或 `${VAR}` → 读宿主启动环境，缺失时报 Invalid：“MCP 服务器 {名称} 引用的环境变量 {VAR} 未设置。”；其余为明文。
6. signature = signature::compute(...)（§4.4）。
```

- 没有启用的提供方且没有 MCP 时，返回空的 `EngineRouting`（`signature` 仍计算，便于判断“从无到有”）。
- 网关方式登记的令牌以 `chat` 为键；同一聊天再次 `prepare` 时替换旧令牌。

### 4.4 签名（`signature.rs`）

SHA-256（`sha2`）十六进制，输入是下列内容按固定顺序拼接的 JSON（字段排序固定，不含任何密钥值或令牌）：

- 每个 `ProjectedProvider`：route、protocol、base_url（直连时为上游地址，网关时为字符串 `gateway`）、headers 名称与值的 SHA-256、models；
- 每个提供方的 revision 与**已启用密钥 id 的有序集合**（不是本次选中的那个，所以轮换不改变签名）。revision 捕获同 id 密钥值的编辑及网关上游地址变化，不读取或散列真实密钥。
- default_model；
- mcp 条目（值取 SHA-256）。

`signature()` 与 `prepare()` 共用同一个计算函数，只是不调用 `next_key`、不登记令牌。

### 4.5 引擎支持的协议

| 引擎 | `OpenaiChat` | `OpenaiResponses` | `AnthropicMessages` |
|---|---|---|---|
| pi | ✅ `openai-completions` | ✅ `openai-responses` | ✅ `anthropic-messages` |
| OMP | ✅ `openai-completions` | ✅ `openai-responses` | ✅ `anthropic-messages` |
| DSH | ✅ `openai-completions` | ✅ `openai-responses` | ✅ `anthropic-messages` |

三者都支持本期的三种协议（01 §3.1），所以本期“引擎不支持的协议自动走网关”（S09）不会触发；这条规则保留在 `prepare` 第 2 步，供以后增加协议时使用。

### 4.6 `catalog`

对 `backend` 执行 `prepare` 第 1–3 步（不取密钥、不登记令牌），过滤掉 `ModelPrefs.hidden`，按提供方 `sort`、模型 `sort` 排序。一个模型在多个协议下都可用时只列一次，`route` 取它 `protocols` 中第一个可用协议的路由。

## 5. routing crate：网关与探测

### 5.1 运行时（`runtime.rs`）

- 进程内只有一个 tokio 多线程运行时（2 个工作线程），在第一次需要时创建（`std::sync::OnceLock`），并在创建时安装 rustls 的 `ring` 提供方（`rustls::crypto::ring::default_provider().install_default()`；已安装时忽略）。
- 同步方法用 `runtime.block_on(...)` 调用异步代码；网关服务任务用 `runtime.spawn` 常驻。
- 上游客户端 `reqwest::Client` 全进程共用一个：`no_proxy()` 不设置（尊重系统代理），连接超时 10 秒，不设整体超时（流式响应可能很长）。

### 5.2 网关（`gateway.rs`）

```rust
pub struct Gateway { /* addr: OnceLock<SocketAddr>, sessions: Mutex<HashMap<String /*token*/, ChatId>>, providers: Arc<dyn ProviderService> */ }
impl Gateway {
    pub fn new(providers: Arc<dyn ProviderService>) -> Arc<Self>;
    /// 第一次调用时绑定 127.0.0.1:0 并启动服务。OpenAI 类返回 `http://127.0.0.1:<端口>/p/<route>/v1`；
    /// Anthropic 返回 `http://127.0.0.1:<端口>/p/<route>`（SDK 自己拼 `/v1/messages`）。所以引擎请求的路径总是 `/p/<route>/v1/...`。
    pub fn base_url(&self, route: &str, protocol: Protocol) -> Result<String>;
    pub fn register(&self, chat: &ChatId) -> Secret;     // 生成 32 字节随机令牌（uuid v4 ×2 拼接），替换该聊天的旧令牌
    pub fn release(&self, chat: &ChatId);
}
```

请求处理：

1. 取令牌：`Authorization: Bearer <令牌>` 或 `x-api-key: <令牌>`；不在 `sessions` 里 → 401 `{"error":{"message":"invalid ThinkRail gateway token"}}`。
2. 解析路径 `/p/<route>/<rest>`：`route` 的前缀 `thinkrail-<短 id>` 找到提供方，后缀 `oc|or|am` 定协议；找不到 → 404。只转发这些 `rest`：
   - `oc`：`v1/chat/completions`、`v1/models`
   - `or`：`v1/responses`、`v1/models`
   - `am`：`v1/messages`、`v1/messages/count_tokens`、`v1/models`
   其他 → 404。
3. 上游地址 = `url::base_url(endpoint, protocol)` 的结果，再按协议拼 `rest`：OpenAI 类去掉 `rest` 开头的 `v1/`（因为地址已含版本）；Anthropic 直接拼（地址已去掉版本）。
4. 密钥：`ProviderService::next_key`（每个请求轮换一次，S10）；提供方没有密钥时不加鉴权头。
5. 请求头：去掉 `authorization`、`x-api-key`、`host`、`content-length`、hop-by-hop 头；Anthropic 加 `x-api-key: <密钥>` 和 `Authorization: Bearer <密钥>`，OpenAI 类加 `Authorization: Bearer <密钥>`；再加提供方的额外请求头；保留引擎发来的 `anthropic-version`、`anthropic-beta`、`content-type`。
6. 请求体原样转发；响应状态、响应头（去掉 hop-by-hop）、响应体流式原样返回。上游连接失败 → 502，体为 `{"error":{"message":"上游不可达：<脱敏原因>"}}`。
7. 日志：`info!(route, key = 标签或 id, status, elapsed_ms)`；不记录请求体、响应体和头部的值。

### 5.3 探测（`probe.rs`）

- `list_models`：`GET {base_url(endpoint)}/models`（OpenAI 类）或 `GET {base_url(endpoint)}/v1/models`（Anthropic）。带 `Authorization: Bearer`；Anthropic 另带 `x-api-key` 和 `anthropic-version: 2023-06-01`。解析 OpenAI 形式 `{data:[{id, context_length?}]}` 和 Anthropic 形式 `{data:[{id, display_name?}]}`；都不是 → `Protocol`：“这不是兼容 OpenAI 或 Anthropic 的模型列表响应。”。超时 15 秒。
- 错误分类（中文，沿用原版 #1084 的措辞）：地址无效、401/403“服务器拒绝了该 API 密钥。”、超时“服务器未及时响应。”、连不上“无法连接到服务器。”、其他状态“服务器返回 HTTP {status}。”、空列表“服务器未返回任何模型。”
- `check`：OpenAI Chat 发 `{"model":id,"messages":[{"role":"user","content":"hi"}],"max_tokens":1}`；Responses 发 `{"model":id,"input":"hi","max_output_tokens":16}`；Anthropic 发 `{"model":id,"max_tokens":1,"messages":[{"role":"user","content":"hi"}]}`。2xx 视为通过；15 秒超时；不重试。
- `check_keys`：对提供方每个启用的密钥，用 `ProviderService::reveal_key` 取值、按模型第一个协议调用 `check`；串行执行，结果按密钥 `sort` 返回。
- 探测不经过网关，直接连上游。

## 6. agents crate：投影

### 6.1 共通

- 新模块 `crates/agents/src/projection_engine.rs`（与现有 `projection.rs` 区分，后者是会话快照投影）：三个引擎的生成函数都放这里，纯函数，便于单测。
- 协议到引擎 `api` 名：`OpenaiChat → "openai-completions"`、`OpenaiResponses → "openai-responses"`、`AnthropicMessages → "anthropic-messages"`。
- 生成的文件都写进 `StartSession.support_dir`，用现有 `host::support_file` 的“内容变化才重写、临时文件 + rename”写法；文件里只有环境变量名，没有密钥、令牌、MCP 值。
- `LaunchSpec.host_env` 的类型从 `Vec<(&'static str, OsString)>` 改为 `Vec<(String, OsString)>`（变量名现在是动态的）；所有调用者一起改。`routing.env` 全部追加到 `host_env`。`command()` 先剔除继承变量（`is_isolated` 已覆盖 `THINKRAIL_*`），再按顺序设置，所以宿主注入的值总是生效。
- `EngineRouting` 为空时不生成任何文件、不加参数（§2.6 回归要求）。
- 用户在档案参数里自己写了会冲突的参数时（pi/OMP 的 `--model`、`--thinking`、`--config`、`-e`；DSH 的 `--patch`），本期不拒绝；宿主参数插在用户参数之后，以 CLI 的“后者生效”为准 `[INFERENCE]`，T7 实测确认。

### 6.2 pi

- 扩展文件 `support_dir/pi-thinkrail.js`，一个文件里同时包含宿主工具（现有 `PI_EXTENSION` 的内容，仅在 `host_tools` 存在时）、提供方注册和 MCP 注册：

```js
export default function (pi) {
  pi.registerProvider("thinkrail-1a2b3c4d-am", {
    baseUrl: "https://api.example.com", api: "anthropic-messages",
    apiKey: "$THINKRAIL_PROVIDER_1A2B3C4D_KEY", headers: { "x-extra": "$THINKRAIL_HDR_1A2B3C4D_0" },
    models: [{ id: "claude-x", name: "Claude X", reasoning: true, input: ["text", "image"],
               thinkingLevelMap: { off: "off", low: "low", high: "high" },
               cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 200000, maxTokens: 16384 }],
  });
  pi.registerMcpServer("thinkrail-context7", { url: "https://mcp.context7.com/mcp", headers: { Authorization: process.env.THINKRAIL_MCP_5E6F7A8B_H0 } });
}
```

- 请求头值和 MCP 的值都通过环境变量传入：provider 的 `headers` 写 `"$<变量名>"`（pi 在加载时解析）；MCP 在扩展里写 `process.env.<变量名>`。文件里不出现值。
- `thinkingLevelMap` 只列模型声明的档位，未声明的写 `null`（Cherry 的教训，02 §4.1）。
- 默认模型：参数追加 `--model <route>/<model_id>`，有力度时再加 `--thinking <档位>`（只在新会话，恢复时引擎沿用会话里的模型）。
- 运行中切换：`set_model {provider, modelId}`，有力度时再发 `set_thinking_level {level}`。

### 6.3 OMP

- 扩展文件 `support_dir/omp-thinkrail.ts`：只注册提供方（`registerProvider`，`apiKey: "THINKRAIL_PROVIDER_<短 id>_KEY"` 不带 `$`，`auth: "apiKey"`，`authHeader: true`），M0 实测在 RPC 模式下生效；宿主工具仍走 `set_host_tools`，不放进扩展。扩展用 TypeScript 写，参数类型声明为最小接口（`registerProvider(name: string, config: Record<string, unknown>): void`），不用 `any`。
- 额外请求头：OMP 的 `headers` 是否把整值当环境变量名解析未验证 `[INFERENCE]`。扩展里改为在加载时读取：`headers: { "x-extra": process.env.THINKRAIL_HDR_1A2B3C4D_0 ?? "" }`，值只在 OMP 进程内存里。
- 覆盖层 `support_dir/omp-thinkrail.yml`：

```yaml
mcp:
  enableProjectConfig: false
modelRoles:
  default: thinkrail-1a2b3c4d-am/claude-x:high   # 只在新会话且有默认模型时写
```

- 参数追加 `-e <扩展>`（有提供方时）和 `--config <覆盖层>`（有 MCP 或默认模型时）。
- MCP：OMP 只能读配置目录（`profile.input.config_root`，未设置时报 `Invalid`：“OMP 档案需要设置配置目录才能使用 ThinkRail 的 MCP 服务器。”）下的 `mcp.json`。宿主**合并写入**（`omp_mcp.rs`）：
  1. 读现有文件（不存在视为 `{}`；JSON 解析失败则不动文件，报 `Invalid`：“{路径} 不是有效的 JSON，ThinkRail 不会覆盖它。”）。
  2. 删除 `mcpServers` 里所有 `thinkrail-` 开头的键，写入本次的条目；其他键和顶层字段原样保留。
  3. 值用 OMP 支持的 `${VAR}` 引用宿主注入的环境变量，不写明文。
  4. 内容没变就不写；变了就临时文件 + rename。
  5. 多个聊天共用同一个 OMP 配置目录、但属于不同项目时，各自的生效集合可能不同，后启动的聊天会覆盖 `thinkrail-` 条目，先启动的聊天在 OMP 下次重载 MCP 时会看到别的项目的集合。本期接受这一点，在 05 中作为已知限制记录；规避办法是给不同项目用不同的 OMP 档案。
- 运行中切换：同 pi（OMP 与 pi 共用同一套 RPC，01 §3.3）。

### 6.4 DSH

- 现有 patch 文件 `dsh-host.patch.yml` 改名为 `support_dir/dsh-thinkrail.patch.yml`，内容按需拼接：

```yaml
- id: tool-todo            # 仅 host_tools 存在时
  disabled: true
- id: llm-pi-ai
  config:
    providers:
      thinkrail-1a2b3c4d-am:
        displayName: "中转 A"
        api: anthropic-messages
        baseURL: https://api.example.com
        apiKeyEnv: THINKRAIL_PROVIDER_1A2B3C4D_KEY
        models:
          - { id: claude-x, name: Claude X, contextWindow: 200000, maxTokens: 16384, input: [text, image],
              reasoningEfforts: { off: null, low: low, high: high } }   # 不推理的模型写 reasoningEfforts: false
- id: acp
  config: { provider: thinkrail-1a2b3c4d-am, model: claude-x }   # 只在新会话且有默认模型时
```

- DSH 0.2.1-rc.5 实测：覆盖层的 `config` **整体替换，不深合并**。适配器读取 profile 与 home 的 `cordis.patch.yml`，按 DSH 顺序选取最后一个 `llm-pi-ai.config`，合入宿主路由；不改用户文件。合并值经 `THINKRAIL_DSH_MERGED_CONFIG` 注入，辅助文件只包含 `!!js` 环境引用，已有 `!!js` 字段保留求值语义。`smoke::dsh_loads_the_projected_provider_default_model_and_mcp_server` 验证宿主与用户路由同时可选。
- DSH 请求头的值：DSH 的 `headers` 不支持环境变量引用 `[INFERENCE]`；本期**不投影额外请求头到 DSH**，有额外请求头的提供方在 DSH 下强制走网关（网关加请求头）。T7 实测后若 DSH 支持再改。
- MCP：`session/new` / `session/resume` 的 `mcpServers` 追加投影条目（stdio：`{name, command, args, env:[{name,value}]}`；http：`{type:"http", name, url, headers:[{name,value}]}`）。DSH 会清掉自己父进程的环境（现有 `acp_server` 的注释），所以值不能靠环境变量传：适配器按 `EnvBinding.var` 从 `routing.env` 取出值，直接放进 JSON-RPC 参数（走 stdin，不落盘、不进日志）。DSH 的 MCP 启动失败会导致会话创建失败，`fail_start` 的提示加上“检查设置 → MCP 服务器”。
- DSH 0.2.1-rc.5 实测要求 stdio MCP `command` 是绝对路径；适配器在传参前按宿主 PATH / PATHEXT 解析命令名，不从工作区查找。找不到时提示填写绝对路径。隔离目录的 `smoke::dsh_loads_the_projected_provider_default_model_and_mcp_server` 已验证提供方、默认力度、运行中切换和 MCP 工具发现，不发模型请求。
- 力度：`session/new` 后，有默认力度时发 `session/set_config_option {sessionId, configId: <category=thought_level 的选项 id>, value: <档位>}`。
- 运行中切换：`session/set_config_option {configId: "model", value: "[\"<route>\",\"<model_id>\"]"}`，再按需设力度。

### 6.5 `ModelInfo.effort`

pi/OMP 从 `get_state` 的 `thinkingLevel` 读；DSH 从 `configOptions` 里 `category == "thought_level"` 的 `currentValue` 读。无法识别的档位名记为 `None`（OMP 的 `inherit`、DSH 表示“提供方默认”的 `""`）。

- 切换后以引擎的回执为准：pi/OMP 在 `set_model`（+ `set_thinking_level`）之后再取一次 `get_state`（OMP 源码 `model-controls.ts` 切模型时会按新模型重新应用档位，所以不能用请求值代替）；DSH 用 `set_config_option` 返回的完整 `configOptions`。目标模型没有 `thought_level` 选项时设力度报 `Invalid`。
- 引擎主动变化也会更新快照：pi/OMP 的 `thinking_level_changed`（pi 字段 `level`，OMP 字段 `thinkingLevel`）、OMP 的 `config_update`；DSH 的 `config_option_update`。
- T9 实测（本机 pi、OMP 18.13.3，隔离配置目录，两个无密钥占位模型，不发 prompt）：RPC `set_model` 不写回 `settings.json` / `config.yml`；OMP 会在 `agent.db` 里记一笔模型使用记录（“最近使用”排序），这不是默认模型。OMP 18.14.1 已核实为 npm 最新版，升级后的 T9 行为待复测。

## 7. app crate

### 7.1 组装（`main.rs`、`services.rs`）

- `Args` 增加 `db_path: PathBuf`：给了 `--data-dir` 时为 `<data-dir>/thinkrail.db`，否则为 `<current_exe 所在目录>/data/thinkrail.db`。
- `Services` 增加字段 `providers: Arc<dyn ProviderService>`、`mcp: Arc<dyn McpService>`、`prefs: Arc<dyn ModelPrefService>`、`router: Arc<dyn ModelRouter>`、`probe: Arc<dyn ProviderProbe>`。
- `build(data_dir, db_path)`：在现有步骤之后 `SqliteSettingsStore::open(db_path)`（`.context("无法打开设置数据库")`），用同一个 `Arc` 填三个字段；`Gateway::new(providers)`；`Router::new(..)`；`Probe::new(providers)`。网关不在启动时监听，第一次 `base_url` 时才绑定端口。
- `host_tools()` 不再设置 `support_dir`；新增 `fn support_dir(&self, chat: &ChatId) -> Result<PathBuf>`，创建并返回 `<data>/engines/<chat>/`。

### 7.2 聊天（`panels/chat.rs`）

- `start_engine` 增加参数 `project: ProjectId`（`ChatPanel::start` 从 `state.workspaces` 里该聊天的工作区取 `project_id`），并在 `runtime.start` 前：
  1. `routing = services.router.prepare(&RouteRequest{chat, project, backend, new_session: binding.is_none()})?`；
  2. `support_dir = services.support_dir(&chat)?`；
  3. 填入 `StartSession`。
- `Chat` 增加 `routing_signature: Option<String>` 和 `restart_pending: bool`。
- 订阅 `AppEvent::RoutingChanged`：对每个有活动会话的聊天，在后台算 `router.signature(..)`；与记录的不同时，空闲则 `dispose` 并清空 `session`（下一条消息重启），运行中则置 `restart_pending`，在回合结束（`phase` 回到 `Idle`）时再 `dispose`。
- `close_chat`：在现有 `dispose` 之后调用 `router.release(&chat)`，并删除 `support_dir`。
- 聊天头部的模型名改为下拉（`Select`）：选项来自 `router.catalog(backend)`；选择后 `run(cx, move || session.set_model(route, model_id, effort), ..)`；`capabilities.model_switch == false` 或没有活动会话时保持只读显示。力度下拉放在模型旁，档位来自所选模型的 `efforts`。

### 7.3 事件

`AppEvent` 增加 `RoutingChanged`。设置页里任何成功的提供方、密钥、偏好、MCP 写操作之后 `state.update(cx, |s, cx| cx.emit(AppEvent::RoutingChanged))`。

### 7.4 设置对话框外壳（`panels/settings/`）

`panels/settings.rs` 拆成目录：

```
panels/settings/mod.rs        # SettingsDialog::open；左侧导航 + 内容区；当前页 enum Page { Models, Providers, Mcp, Agents }
panels/settings/agents.rs     # 现有档案页，原样迁入（含 Form / to_input 及其测试）
panels/settings/models.rs     # 模型页
panels/settings/providers.rs  # 提供方列表 + 编辑对话框 + 密钥管理
panels/settings/mcp.rs        # MCP 页 + 编辑对话框
```

- 对话框宽度改为 832（原版 `max-w-[52rem]`），高度为窗口高度的 80%。
- 左侧导航宽 192、内边距 12，背景 `container_elevated_bg`，右边框 `border_muted`；分组标题“AGENT”高 28、`text::LABEL_PILL`、`text_subtle`；导航项高 28、`px 8`、14px 图标、选中时文字 `primary`、底色 `primary_subtle`。项与图标：模型（`Brain`）、提供方（`KeyRound`）、MCP 服务器（`Plug`）、Agent 档案（`Bot`）。原版的其他分组本期不放（不做空壳）。
- 内容区内边距 16，可滚动；分组之间上边框 + 上下 24；行 `py 12`、下边框，组内最后一行无下边框；行左侧标签最小宽 160。
- 只用 `Roles` 和 `metrics`；缺的图标（如 `EyeOff`）加进 `ui::assets` 的 `icon_assets!`。

### 7.5 模型页（`models.rs`）

| 分组 | 行 | 控件 | 数据 |
|---|---|---|---|
| 默认模型 | 会话模型 | 模型 `Select`（按提供方分组，占位“首个可用”）+ 力度 `Select`（档位来自模型 `efforts`，无档位时禁用） | `prefs.set_default` |
| 默认模型 | 可见模型 | 行内展开（右箭头 ↔ 下箭头）；搜索框；每个模型一行：名称、`{提供方} · {上下文}`、推理模型加“· 推理”、视觉模型加“· 视觉”、`Switch` | `prefs.set_hidden` |

说明文字：“新模型会自动显示。” / “已隐藏 N 个模型。新模型会自动显示。”；空状态“没有可用模型。请先在“提供方”中添加。”；加载失败“无法加载您的默认模型。”+“重试”。

### 7.6 模型服务页（`providers/`，对标 Aether `ProvidersSettings`）

布局为左右两栏。改动不经“保存”按钮，每次编辑立即调用 `update(id, revision, input)`；保存进行中又有新编辑时，等这次完成后用最新的 draft 再保存一次。保存失败时，修改保留在 draft 里，页面显示“保存失败，修改已保留。{错误}”和“重试”。

- 服务商栏（`rail.rs`）：搜索名称；筛选（全部 / 启用 / 停用）。每行显示首字头像、名称（停用时变灰）、状态点（启用且有启用的密钥为绿色，启用但没有密钥为黄色，停用不显示），以及“⋯”菜单（编辑、删除；内置预设不能删）。底部是“新建服务商”。页面加载时调用 `seed_presets`，可重复调用。
- 详情（`detail.rs`）：
  - 标题行：名称；设置按钮打开“服务商设置”（名称、接入方式；新建时另填第一条线路的协议和地址）；右侧是启用开关。
  - API 密钥：没有密钥时显示密码框，回车或失去焦点即添加。有密钥时显示第一个已启用密钥的标签和掩码（启用的密钥多于一个时后面加“+N”），右端是显示 / 隐藏（眼睛）。鼠标移到眼睛上时，左边出现复制（明文只进剪贴板）和编辑（打开 §7.7 并预载这把密钥），边框高亮；指针从眼睛移到这两个按钮上时它们保持显示。钥匙按钮打开 §7.7。预设服务商额外显示“获取密钥”链接。
  - “检测”：对当前线路调用 `probe.check_keys`，每把启用的密钥各请求一次模型列表；没有密钥时不带密钥请求一次。只有一条结果时显示一行；多条时逐行显示，每行带启用开关。
  - 密钥使用方式：轮流使用（只读说明）。
  - API 地址：显示当前线路的地址，失去焦点或回车时保存，下方显示 `GET {models_url}`。有多条线路时，上方是线路选择器。“添加线路”只列出还没用到的协议。设置按钮打开线路设置：协议（下拉框，显示全名，当前项打勾）、地址（预览 `POST {preview}`）、请求头（对整个服务商生效）、设为默认线路、删除线路（二次确认；只在这条线路上的模型一起删除；最后一条线路不能删）。新建服务商时的协议也是同一个下拉框。
- 模型区（`models.rs`）：只列当前线路的模型，按 `model_group` 分组，可整组折叠或移除。工具栏有搜索、能力筛选（视觉 / 推理 / 工具）、“同步模型”和“+”（手动添加模型 ID）。每行显示头像、模型 id、能力图标，以及设置、移除按钮。
  - 同步模型：带第一把启用的密钥调用 `list_models`，然后打开挑选对话框（对标 Aether `ModelPickerDialog`）。标题显示总数；有搜索和“全部添加 / 全部移除”；类型页签（全部、对话、图像、视频、语音合成、语音识别、向量、重排、未知）显示各自的数量；列表按组折叠；每行显示 id、类型下拉、“可看图”和“已选”，以及 + / −。没有“确定 / 取消”：每次点 + / − 或改类型，都立即用 `merge_picked` 合并并保存，保留已有模型的手工字段，并补上缺失的上下文窗口。当前线路已有的模型预先标为已选。
  - 模型类型 `ModelType`（`chat`、`image`、`video`、`tts`、`stt`、`embedding`、`rerank`、`unknown`）：添加模型时由 `model_type(id)` 按关键词推断（照搬 Aether `infer.rs`），用户可以在挑选对话框或模型设置里修改；非对话模型不带上下文和工具推断。只有 `chat` 类型的模型会交给引擎（`routing` 的 `usable`）并出现在默认模型页；其他类型只在设置里显示，行上带类型标签。
  - 模型设置对话框编辑一份副本，确定后按 id 替换原模型。可编辑名称、类型、线路（多选，首项为默认）、上下文窗口、最大输出、视觉 / 推理 / 工具，以及推理档位。开启推理时至少要选一个档位，关闭推理时清空档位。
- 线路规则在 `core::provider_edit`（`set_line`、`retarget_line`、`remove_line`），模型规则在 `core::model_catalog`。界面只做草稿校验（地址格式、请求头不能为空），最终校验在 store。

### 7.7 密钥管理（`providers/keys.rs`）

对话框标题“API 密钥管理”，说明“管理当前服务商的多个 API 密钥，按顺序轮流使用。”。每行显示标签（未填为“API Key”）、掩码，以及复制、编辑、删除和启用开关。底部是标签加密码框（`mask_toggle`），用于“添加 API 密钥”；编辑已有密钥时，密钥框留空表示不修改。页脚显示“N / M 已启用”，没有密钥时显示“暂无 API 密钥”。明文只在“显示”或“复制”时通过 `reveal_key` 取得，切换服务商、写入成功或关闭设置时清掉。复制用 `cx.write_to_clipboard`。

### 7.8 MCP 页（`mcp.rs`）

- 顶部范围 `Segment`：“全局” / “当前项目：{名称}”（没有打开项目时只显示全局）。
- 说明：“服务器在运行 ThinkRail 的这台机器上运行，并继承其环境。项目里的设置覆盖全局。”
- 列表每行：名称、范围标签（全局 / 项目 / 来自全局）、传输方式（stdio / HTTP）、启用 `Switch`（全局视图改全局条目；项目视图里对全局条目改的是项目覆盖，`McpOrigin::GlobalOverridden` 时禁用并提示“已被同名项目条目替换”）、编辑、删除（二次确认：“要移除 {name} 吗？使用它的聊天在下次重启引擎后不再加载它。”）。
- 空状态：“还没有 MCP 服务器。”+ 预设列表（Context7 `https://mcp.context7.com/mcp`、DeepWiki `https://mcp.deepwiki.com/mcp`）+“添加服务器”。
- 编辑对话框：名称（提示“字母、数字、- 和 _。”）、传输方式 `Segment`（stdio 命令 / HTTP URL）；stdio：命令、参数（每行一个，`Textarea`）、环境变量键值行；HTTP：URL（提示“服务器的 streamable HTTP 端点。”）、请求头键值行；值的提示“可以写 $VAR 引用本机环境变量。”；描述（可选）。另有“粘贴 JSON”：接受 `{"mcpServers":{...}}` 或裸映射，逐项预览，只导入通过校验的。

### 7.9 文案

新增 `strings/settings_models.rs`、`settings_providers.rs`、`settings_mcp.rs`（每个页面一个文件，延续 `strings/mod.rs` 的约定），现有 `strings/settings.rs` 保留外壳和档案页的常量。界面上的中文全部来自这些常量。

## 8. 验证

- **单元测试**（每个任务自带，见 05）：存储的 CRUD、约束、revision 冲突、级联删除、迁移与版本过新、轮询顺序与跳过停用、只读路径不推进轮换；地址规范化；签名对轮换不变、对启停密钥变化；三个引擎的生成文件（快照对比）；OMP `mcp.json` 合并保留用户条目；网关鉴权、路径白名单、头部替换、流式转发（本机假上游）。
- **真实引擎**（`#[ignore]`，需要用户同意使用真实中转）：三个引擎各自在直连和网关两种方式下用 easyclaude 跑通一轮、发一张图片能识别、调用一个用户配置的 MCP 工具；两个密钥时网关上游依次收到不同密钥。
- **窗口**：设置四个页面逐项操作并截图；聊天头部切换模型后下一条回复使用新模型。
- 不能算完成的证据：只编译通过、只有单测、只有一张截图（沿用多 Agent 设计包 04 §11）。
