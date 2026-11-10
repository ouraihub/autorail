# Native 可检索日志与 TraceContext 设计

状态：Native 追踪入口、Agent/HostTool 边界、ReviewRunner 与日志页实施中；运行证据见本文末尾。

## 1. 问题

Native 当前有全局 `tracing` subscriber 和 `observability::op()`，但 `op()` 主要包住 UI 发起的单个后台任务。日志能看到 `op{id, at}`，却不能稳定回答：

- 哪个用户动作触发了这次 host tool 调用？
- 同一个操作经过了哪个 app service、router、workspace、engine？
- 一个 Agent turn 的启动、tool call、结束、错误能否用一个 id 搜索？
- 跨 GPUI background executor、async task、thread、engine reader 后，父级 trace 是否仍然存在？

MES.Backend 的目标是 Controller → Service → Repository 全链路同一 TraceId。Native 没有 HTTP Controller/DAO 层，但需要同一原则：

```text
用户动作 / host tool / engine turn
    └─ TraceContext
        ├─ UI handler
        ├─ background job
        ├─ service / router / workspace
        ├─ host tool bridge
        ├─ external agent process
        └─ completion / error / cleanup
```

## 2. 设计目标

### 必须实现

- 每个可定位的顶层动作有稳定 `trace_id`；
- 子操作有独立 `span_id` 或 tracing span，但保留 `trace_id`；
- 同一上下文跨 background executor、async channel 消费和显式 thread 传播；
- 结构化字段可被日志 tail、搜索和未来 JSON sink 使用；
- 日志包含 `trace_id`、`span_id`、`parent_span_id`、`operation`、`chat_id`、`workspace_id`、`backend`、`component`；
- 请求结束、失败、取消、超时都留下终态日志；
- trace id 可回显给用户或复制到问题报告中；
- 敏感参数、API key、完整 prompt、完整文件内容不进入日志。

### 不做

- 第一阶段不引入 OpenTelemetry exporter、Jaeger、OTLP 或网络 tracing backend；
- 不把所有日志文本改成用户 locale；日志字段名和诊断消息保持稳定英文/现有语言；
- 不要求每个纯函数创建 span；纯逻辑只在跨边界或有耗时/失败风险的操作处埋点；
- 不用 `chat_id` 替代 `trace_id`。一个聊天包含很多独立动作，一个动作也可能产生多个内部 span。

## 3. 与 MES.Backend 的对应关系

| MES.Backend | Native 对应 |
|---|---|
| HTTP request | UI action、host tool call、engine turn、review run |
| `TraceMiddleware` | `TraceContext::root()` / `observability::operation()` |
| `Activity.Current` | 当前线程/任务显式持有的 `TraceContext` + tracing span |
| `ILogger.BeginScope` | tracing span fields；跨线程通过 `TraceContext::enter()` 恢复 |
| Controller | ChatPanel handler、HostToolHandler、bridge request handler |
| Service | app service、ReviewRunner、routing、workspace operation |
| Repository | settings-store、plan-review store、git operation |
| `TraceInterceptorBase` | 共享边界函数的手工 `trace_span!`，不做反射式全量拦截 |
| `X-Trace-Id` | 日志页、错误提示、评审事件和可复制诊断信息中的 trace id |

MES 当前的价值在于统一字段和跨层传播；Native 不照搬 C# DispatchProxy，而使用 Rust tracing 的结构化 span。

## 4. TraceContext 契约

`crates/observability` 复用固定 Git revision 的 `lumakit-observability`，不复制第二套 ID/operation 实现：

```rust
impl TraceContext {
    pub fn root(operation: &'static str) -> Result<(Self, tracing::Span), String>;
    pub fn child(&self, operation: &'static str) -> tracing::Span;
    pub fn trace_id(&self) -> &str;
}
```

随机源失败只记录追踪不可用，不重试生成、不伪造 ID，也不让日志故障阻断业务。跨线程持有并重新进入真实 span；子 span 的可见级别不得使 INFO 事件失去根关联。

实现约定：

- `trace_id` 使用 32 个小写十六进制字符；
- root trace id 由 `uuid` 或随机源生成，不使用自增数字作为对外诊断 id；
- span id 由 tracing subscriber 生成；若底层 subscriber 没有 span id，也必须保留 trace id；
- `TraceContext` 可安全 clone，禁止全局可变“当前 trace id”；
- `TraceGuard` 只负责进入/退出 span，不负责业务状态；
- `operation()` 是现有 `op()` 的兼容替代或扩展，不能同时产生两个互相独立的根 id。

建议日志字段：

```text
trace_id
span_id
parent_span_id
operation
component
workspace_id
chat_id
backend
request_kind
result
error_code
duration_ms
```

字段名使用 snake_case，便于 text/json sink 和 grep 查询。

## 5. 顶层 Trace 根

### 5.1 UI 与 AppState 后台任务

`state::run()` 创建 root trace：

```text
operation=ui.background
request_kind=<caller operation>
```

它把 `TraceContext` 和 tracing span 一起 move 到 background executor，并在 completion callback 重新 enter。同一个 trace 包住：

- job 开始；
- service/router/workspace 日志；
- completion callback；
- 成功、失败或 view 已关闭终态。

现有 `op{id, at}` 字段保留一段迁移期，但新增 `trace_id` 后不再把自增 op id 当作全链路主键。

### 5.2 Host tool call

HostToolHandler 的入口为每个调用创建 root 或 child：

```text
operation=host_tool.call
request_kind=<tool name>
chat_id
workspace_id
```

Bridge 线程、MCP worker thread 和 OMP in-process worker 必须把 context clone 到执行闭包。日志至少包含：

```text
trace_id chat_id workspace_id tool duration_ms is_error
```

`request_review` 产生的 Reviewer session 使用：

```text
operation=review.run
parent_span_id=<host tool or button span>
chat_id=<worker chat>
reviewer_chat_id=<temporary reviewer chat>
item_id
backend
```

### 5.3 Engine session / turn

启动、prompt、settled、stop、dispose 使用同一 chat 级别的 session context，并为每个 turn 创建 child span：

```text
operation=agent.turn
chat_id
backend
```

Engine reader 线程收到事件时不依赖“当前线程上下文”；事件结构或 session owner 必须携带 context，再在处理点 enter。

### 5.4 外部进程与请求

Native 不能控制 pi/OMP/DSH 内部日志，但必须在自身边界记录：

- process start：executable 不含 secret，backend、pid、trace_id；
- process exit：exit code、duration、trace_id；
- protocol request/reply：method/type、request id、duration、trace_id；不记录完整 payload；
- engine error：稳定 error code 和脱敏摘要；
- cleanup：router release、support directory removal、session dispose。

## 6. 跨线程与异步传播

### 6.1 明确传播

以下边界必须显式携带 `TraceContext`：

- `state::run()` background job；
- `std::thread::spawn` / `thread::Builder`；
- `async_channel` 发出的事件；
- reviewer private `SessionSink`；
- `ReviewEvent`；
- MCP/bridge worker call；
- terminal/background worker。

禁止依赖线程本地变量推断父级关系。Rust `tracing` 的 span 可在线程间使用，但必须 clone/传递 context 并在目标闭包中 `enter`。

### 6.2 Event 结构

应用内部的评审 channel 使用 `(tracing::Span, ReviewEvent)` envelope。生产者携带当前 review span，工作台消费时重新进入；私有 ReviewerSink 持有 session span。`core::ReviewEvent` 不依赖 subscriber 或 LumaKit，保持领域 DTO 边界。

Agent 由 process/session owner 显式持有上下文，在 reader、请求回复和释放边界恢复。跨进程只记录宿主可观察生命周期，不声称能追踪外部引擎内部实现。

## 7. 日志输出与搜索

第一阶段继续使用当前 daily file + tail UI，不改变日志目录和保留策略。输出层新增稳定字段：

```text
2026-11-07T... INFO trace_id=... span_id=... operation=host_tool.call component=host_tools chat_id=... tool=todo_update message="..."
```

text 输出必须保持 grep 友好。未来增加 JSON layer 时，字段名无需改变。

日志页增加：

- 按 `trace_id` 过滤；
- 复制 trace id；
- 当前条目显示 operation/component；
- 没有 trace id 的启动日志仍可显示。

外部错误、用户可见错误和诊断报告都可以附带：

```text
诊断 ID：<trace_id>
```

但不得把完整异常堆栈直接显示给普通用户。

## 8. 分层日志规范

每个跨边界方法按统一结构记录：

```text
ENTRY  level=debug  operation/component + stable input summary
EXIT   level=debug/info operation/component + duration_ms + result summary
ERROR  level=warn/error operation/component + duration_ms + error_code
CANCEL level=info operation/component + reason
```

参数摘要必须脱敏：

- API key/token/password/secret → 不记录；
- prompt → 只记录长度和 hash（如确有需要）；
- 文件内容 → 只记录路径、大小、hash；
- 大型 JSON → 只记录字段名/计数/稳定摘要；
- 用户输入 → 默认不记录全文。

Rust 代码优先使用结构化字段：

```rust
tracing::info!(
    trace_id = %trace.trace_id,
    operation = "workspace.git_commit",
    workspace_id = %workspace,
    duration_ms,
    "operation completed"
);
```

## 9. 实施顺序

1. 在 observability 增加 `TraceContext`、root/child span 和稳定字段格式；
2. 改造 `state::run()`，让 context 跨 background job 和 completion callback 传播；
3. 改造 HostToolHandler、bridge、MCP、OMP worker，给每次 tool call 建立/继承 trace；
4. 改造 agent process/session 边界，记录 start/request/reply/exit/cleanup；
5. 改造 ReviewRunner 和 ReviewEvent，传递 review trace；
6. 改造 workspace/routing/plan-review/settings-store 的跨边界操作日志；
7. 日志 tail parser 增加 trace/span/operation 字段；
8. 日志页增加 trace filter/copy；
9. 为每个公共边界增加一条 trace propagation 测试；
10. 最后删除或降级旧 `op{id}` 专用解析逻辑。

## 10. 验收

- 一个 UI 发起的后台操作，能用同一 `trace_id` 找到 job、service、store、completion 和 error；
- 一个 host tool call，能找到 bridge、tool handler、workspace/plan write 和返回结果；
- 一个 Agent turn，能找到 process request、session event、settled/error 和 cleanup；
- Reviewer 能找到 worker chat、temporary reviewer chat、verdict application 和 cleanup；
- 跨线程后 trace id 不丢失，不错误串到另一个 chat/request；
- 取消、超时、异常都有终态日志；
- 敏感信息不出现在日志；
- text tail 与未来 JSON sink 使用相同字段名；
- 测试覆盖 root、child、thread、async channel、cancel、failure 和并发请求隔离。
