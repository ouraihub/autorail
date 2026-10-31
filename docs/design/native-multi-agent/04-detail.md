# 04 详细设计

本文规定目标接口与行为。类型与 trait 的源码以 `crates/core` 为准；本文与代码不一致时，先修正本文，再改代码。

## 1. 服务契约（`crates/core`）

### 1.1 强类型 id

`ProjectId`、`WorkspaceId`、`ProfileId`、`ChatId`、`NativeSessionKey`、`TerminalRuntimeId`、`ReviewId`、`PlanItemId` 都是 `#[serde(transparent)]` 的字符串 newtype，类型之间不能混用。

- `ChatId` 是宿主生成的聊天身份。
- `NativeSessionKey` 是引擎的原生 locator（pi / OMP 的 session 文件路径，DSH 的 sessionId），对宿主不透明，只在恢复会话时使用。

### 1.2 trait 一览

| trait | 职责 | 实现（services） |
|---|---|---|
| `WorkspaceService` | 项目、worktree、受限文件读取、Git | `GitWorkspaceService` |
| `ProfileService` | 档案版本、默认档案、聊天绑定 | `JsonProfileStore` |
| `PlanReviewService` | 计划项、审查评论、审查锚点 | `JsonPlanReviewStore` |
| `TerminalService` | PTY 生命周期、标签目录、回放 | `PtyTerminalService` |
| `AgentRuntime` / `AgentSession` | 引擎进程协议 | `PiRuntime`、`OmpRuntime`、`DshRuntime` |
| `SessionSink` / `TerminalSink` | 后台线程向界面推送事件，不阻塞 | app 侧的 channel 实现 |
| `HostToolHandler` | 宿主工具调用分发 | `HostTools`（组合 `PlanReviewService`） |

约定：

- **阻塞语义**：所有服务方法都是同步阻塞的，必须在后台执行器中调用（`cx.background_spawn`），不能在 GPUI 主线程上调用。
- **并发**：所有 trait 都是 `Send + Sync`，可以用 `Arc<dyn …>` 共享。实现内部自己负责互斥：持久化用 `parking_lot::Mutex` 加 fs2 文件锁，引擎会话用状态锁。
- **流式输出**：`AgentSession` 的方法只等到引擎**受理**命令就返回，完成和流式输出一律走 `SessionSink`。

### 1.3 会话事件与 revision

`SessionSnapshot` 是聊天的权威状态，每次变化 `revision` 加一。revision 取自进程级单调计数器，所以同一聊天重启引擎后的新会话也一定比旧会话大。`SessionEvent` 分为 `Snapshot`、`Message`（按 id 插入或替换）、`Delta`（向消息的第 `block` 个文本或思考块追加文本）、`Error` 四种，每个事件都带当时的 revision。界面只接受 revision 不小于当前值的快照，所以晚到的快照不会覆盖已经收到的新增量。

## 2. 错误规范

### 2.1 分层

- **库层**（core、services）：使用 `core::Error`（`thiserror` 定义）。不得返回 `String` 错误、`Box<dyn Error>` 或 `anyhow`。
- **应用层**：`main` 和启动流程用 `anyhow::Result`，用 `.context()` 补充上下文。视图把 `core::Error` 转成界面上的提示。
- **禁止**：非测试代码中的 `unwrap` / `expect`（clippy `unwrap_used`、`expect_used` 为 warn，CI 当作错误处理）。锁中毒：改用 `parking_lot`，它不会中毒。`let _ =` 吞掉错误只允许出现在 Drop 和清理路径中，并且要记一条 `tracing::debug!`。

### 2.2 错误类型

`core::Error` 由 `kind` 和人类可读的上下文组成，`code()` 返回稳定的错误码。界面和日志按 code 分支处理，不解析消息文本。

| code | 含义 | 典型来源 |
|---|---|---|
| `NOT_FOUND` | 对象不存在 | 项目、工作区、档案、评论 |
| `INVALID` | 输入不合法 | 路径越界、参数格式 |
| `CONFLICT` | 乐观并发冲突，需要重新读取 | 档案 `expectRevision` 不匹配 |
| `BUSY` | 另一个写入者持有锁 | fs2 锁竞争 |
| `CORRUPT` | 持久化数据损坏，未做修改 | JSON 无法解析或违反不变量 |
| `FORBIDDEN` | 操作被安全规则禁止 | 删除默认工作区、越过 symlink |
| `TIMEOUT` | 有界操作超时 | Git 30 秒、握手 30 秒 |
| `EXTERNAL` | 外部命令失败 | Git 返回非零 |
| `PROFILE_DISABLED` | 档案已禁用 | 新建聊天 |
| `BACKEND_UNAVAILABLE` | 引擎无法启动或握手失败（附 stderr 摘要） | 可执行文件不存在、OMP 没有配置默认模型 |
| `PROTOCOL` | 引擎协议违约 | 帧格式错误、分片序列错误 |
| `SESSION_CLOSED` | 会话已关闭 | 已退出的进程收到 prompt |
| `SESSION_BUSY` | 会话正在运行，拒绝新的 prompt | 运行中再次发送 |
| `INTERACTION_EXPIRED` | 交互已经不再等待回答 | 重复回答、超时 |
| `IO` | I/O 失败，保留 `source` | 文件读写 |

新增 code 的条件：调用方确实需要区别处理。不能为每一处失败都单独加一个码。

### 2.3 消息内容

- 错误消息面向用户，使用中文，说明“什么失败了”以及“可以怎么做”。
- 消息中**禁止**出现凭据、环境变量的值、prompt 正文、工具参数和完整的 stderr。stderr 最多截取 2 KiB 摘要，并先脱敏：名字含 `key|token|secret|password|authorization` 的 `名字=值` / `名字: 值` 把值替换为 `***`；提到这些词的其他行里，形似密钥的词（≥16 位、含数字的字母数字串）替换为 `***`。“No API key found” 这类说明文字保持可读，否则用户看不到真正的失败原因。

## 3. 日志规范

- **库**：`tracing`。`services` 只使用宏（`info!`、`warn!` 等），不安装 subscriber。
- **初始化**：`app::logging::init(data_dir)`，在第一个窗口打开之前执行。
  - 输出到 `<data>/logs/thinkrail-native.YYYY-MM-DD.log`，按日滚动，使用 `tracing-appender` 的非阻塞 writer，`WorkerGuard` 在 `main` 中持有到进程退出。
  - 默认级别 `info`，可用环境变量 `THINKRAIL_LOG` 覆盖（`EnvFilter` 语法，例如 `thinkrail_agents=debug`）。
  - debug 构建同时输出到 stderr。
- **级别**：

  | 级别 | 用途 |
  |---|---|
  | `error` | 需要用户处理、导致功能不可用的失败 |
  | `warn` | 已降级或已恢复的异常，例如引擎崩溃后会话被关闭 |
  | `info` | 生命周期：应用启动与退出、项目打开、引擎启动与退出（带退出码）、PTY 启动与退出 |
  | `debug` | 协议帧的类型和大小、锁等待、请求耗时 |
  | `trace` | 不使用（不记录帧内容） |

- **结构化字段**：统一使用 `chat`、`workspace`、`backend`、`runtime`、`code`、`elapsed_ms`。span 覆盖一次服务调用，例如 `#[instrument(skip_all, fields(workspace = %id))]`。
- **禁止记录**：凭据、环境变量值、prompt 和消息正文、工具参数和结果、文件内容、终端输出。路径只在 `debug` 级别记录。
- **`print!` / `eprint!`**：clippy 中 deny / warn，只有日志初始化之前的启动失败可以写 stderr。

## 4. 引擎适配（`crates/agents`）

### 4.1 公共部分

每个后端一个模块：`pi`、`omp`、`dsh`，三者互不调用。共享部分：`process`（启动、环境、进程树）、`framing`（JSONL 与 OMP 分片）、`projection`（有界投影与 revision）、`session`（请求关联、读线程、启动失败与退出）；`rpc` 只由 pi 和 OMP 共用（两者的 `AgentMessage`、流式事件和 `extension_ui_*` 对话框格式相同）。

- **进程启动**：档案只支持 `Launch::External { executable, args }`。`executable` 必须是存在的绝对路径。
- **环境**：继承的环境先剔除凭据类变量（含 `API_KEY`、`TOKEN`、`SECRET`、`PASSWORD`、`CREDENTIAL`、`ACCESS_KEY`、`PRIVATE_KEY`，以 `_BASE_URL` 结尾，以及 `AWS_`、`OPENAI_`、`ANTHROPIC_` 等供应商前缀）和引擎配置变量（`PI_*`、`OMP_*`、`DSH_*`、`THINKRAIL_*`），再注入：
  - 配置目录：pi 和 OMP 用 `PI_CODING_AGENT_DIR`，DSH 用 `DSH_HOME`；
  - 档案中 `envRefs` 引用的变量，只按名称从本应用的启动环境复制；
  - 宿主工具连接参数 `THINKRAIL_HOST_PIPE` 和 `THINKRAIL_HOST_TOKEN`。
- **Windows 进程**：使用 `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP`，退出时按 PID 终止整个进程树，绝不按进程名查杀。
- **帧读取**：JSONL 单帧上限 64 MiB（OMP 单行 1 MiB，见 §4.3），超限视为协议错误。stdout 读线程、stdin 写线程、stderr 读线程、退出等待线程相互独立。stderr 只保留最后 64 KiB，用于生成脱敏摘要（最多 2 KiB）。
- **握手**：`start` 最多等待 30 秒，必须拿到真实的 `NativeSessionKey` 才算成功。失败时结束进程，返回 `BACKEND_UNAVAILABLE`，并附带脱敏后的 stderr 摘要。
- **投影**：消息投影有上限（2000 条或 8 MiB，超出时丢弃最早的消息）。单条消息超过 256 KiB 时按块截断，并设置 `output_truncated`。引擎自己仍保留完整历史。一轮结束时仍为 running 的工具卡标为 failed。
- **对话框时限**：pi / OMP 带 `timeout` 的对话框由引擎自己到期处理；宿主到期后移除卡片，迟到的回答返回 `INTERACTION_EXPIRED`。
- **退出**：先取消待答交互，再关闭 stdin，3 秒内未退出就终止进程树。引擎意外退出时会话转为 `Closed`，`last_error` 附退出码和 stderr 摘要。

### 4.2 pi（`--mode rpc`）

- **参数**：`args` 后面追加 `--mode rpc`；启用宿主工具时再追加 `-e <宿主工具扩展>`（§6.2）；恢复会话时再追加 `--session <locator>`。`args` 中出现 `--mode`、`--session`、`--session-id`、`--no-session`、`--continue`、`--resume`、`--fork`、`--print` 时拒绝启动。
- **握手**：`get_state` 取 `sessionFile` 作为 locator，再用 `get_messages` 回放历史。空会话可能还没写盘（懒持久化），所以空会话不能立刻恢复。
- **发送**：`prompt` 的 response 只表示受理（`disposition` 为 `handled` 时是扩展命令在本地完成，直接回到 `Idle`）。运行中为 `Running`，一直等到 `agent_settled` 才回到 `Idle`；`agent_end` 不代表完成。
- **流式**：pi 1.4 的 `message_update` 不带完整消息，只有 `assistantMessageEvent`（`*_start` / `*_delta` / `*_end`，按 `contentIndex` 定位块）；`message_end` 的完整消息为准。消息没有 id，宿主在 `message_start` 生成。
- **交互**：`extension_ui_request` 中 select / confirm / input / editor 映射为 `Interaction`，回复 `extension_ui_response`。取消时回复 `cancelled:true`。
- **停止**：发送 `abort`。
- **能力**：resume、history_replay、token_deltas、questions 为 true；`tool_permissions` 为 false，因为 pi 不会在每次工具调用前请求审批。

### 4.3 OMP（`--mode rpc-ui`）

- **参数**：`args` 后面追加 `--mode rpc-ui --approval-mode always-ask`；恢复会话时再追加 `--session <locator>`。`args` 中出现 `--approval-mode` 或 pi 同样的会话类参数时拒绝启动。
- **握手**：
  1. 等待 `ready` 帧，要求它声明支持协议 v2，然后发送 `negotiate_protocol` 选择 v2；
  2. 发送 `set_event_filter { events: null, messageUpdates: "delta" }`，`message_update` 不再携带整条消息快照，按块增量投影（与 pi 共用转换）；
  3. 发送 `set_ask_dialog { enabled: true }`；
  4. 启用宿主工具时发送 `set_host_tools`（§6.3）；
  5. 发送 `get_state` 取 `sessionFile` 作为 locator，再用 `get_messages_page`（每页 256）回放历史，最多保留最新 2000 条。
  - 如果进程退出并且 stderr 含 “No default model selected”，返回 `BACKEND_UNAVAILABLE`，提示用户在 OMP 配置目录里配置模型。
- **消息 id**：OMP 的消息帧带 `messageId`，直接用作投影 id。
- **完成**：以 `prompt_result` 或 `session_settled` 为准；`agent_end` 不代表完成。
- **分片**：v2 的 `rpc_chunk` 必须按 chunkId 顺序连续重组，总长度不超过 64 MiB。顺序中断、长度不一致或 base64 错误都按协议错误处理。
- **交互**：select / confirm / input / editor / ask 映射为 `Interaction`。`ask` 的回答用 `answers`。其他 UI 方法（`setWidget`、`notify`、`setStatus` 等）一律忽略。
- **工具审批**：在 `always-ask` 模式下，OMP 会用 select 询问是否运行工具，这些请求按普通交互展示给用户。对宿主自己的工具，**不自动批准**，同样展示给用户；用户可在设置中为宿主工具开启“始终允许”，开启后才在应用侧自动回答。
- **能力**：resume、history_replay、token_deltas、questions、tool_permissions 全部为 true。

### 4.4 DSH（ACP）

- **参数**：在用户 `args`（CLI 入口路径）之后插入 `--profile acp`；启用宿主工具时再插入 `--patch <数据目录>/engines/dsh-host.patch.yml`（内容 `- id: tool-todo` / `disabled: true`，由宿主的 TODO 工具替代）。这些参数必须插在 app 参数之前。用户参数中已经有 `--profile` 时拒绝启动。没有配置目录时使用 DSH 默认的 `~/.dsh`；目录不存在时拒绝启动。
- **进程 cwd**：设为 `DSH_HOME`，不设为工作区。DSH 会读取进程 cwd 下的 `.env`；工作区只通过 `session/new.cwd` 传入，并且宿主要先校验目录存在。
- **握手**：`initialize { protocolVersion: 1, clientCapabilities: {} }`。检查 `agentCapabilities.sessionCapabilities` 中 `resume` / `close` 键是否存在。然后 `session/new { cwd, mcpServers: [宿主工具] }`，用返回的 `sessionId` 作为 locator。恢复时调用 `session/resume { sessionId, cwd, mcpServers }`；它的返回中没有 `sessionId`，locator 保持请求里的值。
- **模型**：`configOptions` 中 `category: "model"` 项的 `currentValue` 是 JSON 编码的 `[provider, model]`，显示名从 `options` 按 value 查找。`category: "thought_level"` 是推理档位。
- **消息**：`session/update` 中的 `agent_message_chunk` 和 `agent_thought_chunk` 是已提交的完整块，不是 token delta；同一 `messageId` 的多个块依次追加到同一条消息。`tool_call` 和 `tool_call_update` 映射为工具卡。DSH 不回显用户消息，宿主在发送时自己插入。
- **完成**：`session/prompt` 的回复即本轮结束（`prompt` 只等写入成功，不阻塞到回复）。**停止**：先把待答权限请求回复为 cancelled，再发送 `session/cancel` notification。**退出**：`session/close`，最多等 10 秒，然后关闭 stdin。
- **权限**：`session/request_permission` 映射为 `Permission` 交互，只能回复请求中给出的 `optionId`；用户取消时回复 `{ outcome: "cancelled" }`。其他会话的请求直接回复 cancelled。未知的引擎请求回 `-32601`。
- **能力**：`history_replay`、`token_deltas`、`questions` 为 false；`resume` 以 initialize 的声明为准；`tool_permissions` 为 false，因为只有返回 ask 的工具才会请求权限。
- **版本**：目前只有预发布版（npm `latest` 为 0.2.1-rc.5，`alpha` 为 0.2.2-alpha.2），界面上显示“预发布”。

## 5. 工作区、Git、文件、终端

- **路径约束**：所有入参都是工作区相对路径。逐段 canonicalize 后必须仍在 worktree 内；不允许经过指向外部的 symlink 或 junction；不允许 `.git`（不区分大小写，忽略结尾的 `.` 和空格）；不允许 NTFS 备用流（`:`）。
- **Git**：直接执行 `git` 可执行文件，不经过 shell。单条命令 30 秒超时，输出上限 16 MiB。传给 Git 的路径去掉 `\\?\` 前缀。diff 使用 `--no-ext-diff --no-textconv`；未跟踪文件用 `diff --no-index` 生成；symlink 只显示链接目标文本，不读取目标内容。
- **worktree**：新建的 worktree 放在 `<data>/worktrees/<project-slug>/<branch>`，记录归属信息（路径、分支、commonDir）。删除前逐项核对归属，并检查不存在 reparse point；不强制删除，也不递归删除任意路径。默认工作区和外部工作区不能删除。
- **文件**：只读，上限 16 MiB。含 NUL 或非 UTF-8 的内容返回 `FileContent::Binary { byte_len }`，不做有损解码。
- **终端**：使用 portable-pty。shell 只能显式选择 `powershell` / `pwsh` / `cmd`，缺失时报错，不静默回退到其他 shell。历史保留 128 KiB UTF-8，并带字节偏移；attach 在锁内同时取回放快照和结束偏移，界面丢弃偏移已经包含在回放里的实时块。事件队列有界，满了就阻塞读线程施加背压。结束进程以等待线程观察到实际退出为准（portable-pty 0.9.0 的 kill 返回值不可靠）。

## 6. 宿主工具（TODO / 审查）

### 6.1 工具核心

`services::host_tools::HostTools` 实现 `HostToolHandler`，参数和结果都是 JSON（工具调用本来就是 JSON 契约）。作用域是 `(WorkspaceId, ChatId)`，所有状态都委托给 `PlanReviewService`，不另外保存一份。

工具清单沿用原版语义：

| 工具 | 参数 | 语义 |
|---|---|---|
| `todo_list` | `status?`、`offset?` | 按分组列出，超长时分页 |
| `todo_add` | `title`、`group?`、`after?`、`note?` | 新项为 `pending`，`origin: agent` |
| `todo_update` | `id`、`status?`、`title?`、`note?`、`summary?`、`verification?`、`commitSubject?` | 设为 in_progress 时，其余 in_progress 降回 pending 并作为 paused 返回；离开 done 时清除该项的 summary / verification / commitSubject |
| `todo_promote` | `id`、`title?` | 把松散项原子地提升为空分组，分组继承原 id |
| `todo_remove` | `id` | 只在用户要求时使用（写在工具描述里，代码不强制） |
| `todo_write` | `groups[]` | 只协调 `origin: agent` 的项；用户项和 done 项保留 |
| `todo_plan_summary` | `summary` | 计划级摘要，写成 PR 正文格式 |
| `resolve_comment` | `commentId`、`note?` | 只能解决本聊天中已发送的评论；草稿评论不能通过工具解决 |
| `request_review` | `itemId` | 启动只读审查（§6.4） |

不变量：同时最多一个 in_progress；done 项不可删除；分组状态由组内各项派生，不单独存储。按用户决定沿用原版：`todo_update` 和 `todo_remove` 不检查 origin，Agent 可以修改用户添加的项。

### 6.2 MCP server（pi、DSH）

`thinkrail-native.exe mcp-host` 子命令是 stdio MCP server：stdout 只输出协议帧，日志写 stderr。

- 方法：
  - `initialize`：客户端请求的版本在受支持列表 `2025-11-25 / 2025-06-18 / 2025-03-26 / 2024-11-05` 中就原样回显，否则回 `2025-11-25`。返回 `capabilities: { tools: {} }` 和 `serverInfo`。
  - `notifications/initialized`、`ping`、`tools/list`（不分页）、`tools/call`（返回 `{ content: [{ type: "text", text }], isError?, structuredContent? }`）、`notifications/cancelled`。
  - 未知方法返回 `-32601`。
- 子进程启动后，用 `THINKRAIL_HOST_PIPE` 连接主进程的命名管道，第一帧发送 `THINKRAIL_HOST_TOKEN`。主进程用令牌查到对应的聊天，令牌无效就立即断开。每次引擎启动都签发新令牌。
- **pi 接入**：启动时把固定扩展写到 `<数据目录>/engines/pi-host-tools.js`（内容不变就不重写），用 `-e` 加载：`pi.registerMcpServer("thinkrail", { command: process.env.THINKRAIL_HOST_EXE, args: ["mcp-host"], exposure: "direct", timeout: 600 })`。`THINKRAIL_HOST_EXE` / `PIPE` / `TOKEN` 设在 pi 进程环境上，pi 的 stdio MCP 子进程继承它们。这是 pi 官方的按会话注册方式，不写入用户配置，也不影响用户单独运行 pi。必须使用 `exposure: "direct"`：默认的 codemode 不会把工具直接声明给模型。pi 在 `session_start` 就连接 MCP server。
- **DSH 接入**：`session/new` 和 `session/resume` 的 `mcpServers` 都传 `{ name: "thinkrail", command: <exe 绝对路径>, args: ["mcp-host"], env: [{ name, value }] }`。DSH 会清洗父进程环境，管道名和令牌必须显式放进 `env`。
- 宿主工具是增强能力：命名管道起不来时记 `warn`，聊天照常启动，只是没有宿主工具。

### 6.3 OMP 宿主工具

握手阶段发送 `set_host_tools`，每个工具都带 `loadMode: "essential"`；不设的话，非内置工具默认是 discoverable，不会出现在顶层工具列表里。

- 收到 `host_tool_call { id, toolCallId, toolName, arguments }` 时，在独立线程执行工具核心（不阻塞读线程），回复 `host_tool_result { id, result: { content }, isError? }`。OMP 直接在宿主进程内调用，不经过 `mcp-host`。
- 收到 `host_tool_cancel { targetId }` 时，丢弃该调用迟到的结果（工具核心是同步的短操作，不中途打断）。
- 工具名不得与 OMP 内置工具重名。

### 6.4 `request_review`

审查者是同一档案下新建的一个只读聊天，工具只开放 read、grep、find。它输出一个 fenced json 块，内容为 `PlanReviewResult { itemId, verdict: approve | request_changes, summary?, findings[] }`；`request_changes` 必须至少包含一条 finding。

- pi 和 OMP 同步等待结果。pi 的 MCP 超时设为 600 秒，并通过 progress 通知重置计时。
- DSH 的 MCP 调用固定 60 秒超时，改为异步：先立刻返回“审查已开始”，审查完成后，把结论作为一条 follow-up prompt 发回原聊天。
- 审查结论驱动的自动修复最多一轮。

## 7. 持久化

数据目录：`--data-dir`，默认 `%USERPROFILE%\.thinkrail-native`。不读取、也不导入 `~/.thinkrail`。

| 文件 | 内容 | 写入方式 |
|---|---|---|
| `workbench.json` | 项目、工作区、worktree 归属 | 进程内锁 + 原子 rename |
| `agent-profiles.json` | 档案版本（只追加）、默认档案、聊天绑定 | fs2 独占锁内重读 + 原子 rename |
| `review-plan.json` | 每个 `(workspace, chat)` 的计划项和评论 | fs2 独占锁内重读 + 原子 rename |
| `terminals.json` | 终端标签目录（不含运行时） | fs2 独占锁内重读 + 原子 rename |
| `logs/` | 按日滚动的日志 | tracing-appender |

每个文件都带 `schemaVersion`。读到未知版本或损坏的数据时返回 `CORRUPT`，原文件保持不动。

## 8. 设置与新聊天

- **设置**是对话框，从顶栏右侧齿轮打开，和原版一样。“Agents”页列出档案，每项显示：后端（pi / OMP / DSH，DSH 标“预发布”）、名称、配置目录、启用状态、默认标记。可以新建、编辑（保存为新版本）、删除、设为默认。
  - 编辑只影响新聊天。详情中注明“已有聊天继续使用版本 N”。
  - 档案的必填项：可执行文件绝对路径和 JSON 字符串数组参数，界面给出三种后端各自的填写示例。宿主自动追加的参数（`--mode` 等）不需要用户填写。
- **新聊天**默认使用默认档案。聊天头部始终显示后端、档案和模型。没有默认档案时提示先去设置。档案缺失或已禁用的聊天保留历史，但不能发送，也不会自动换用其他后端。
- **关闭聊天**：标签上的 × 关闭聊天。宿主先释放引擎进程，再从 `agent-profiles.json` 删除该聊天的绑定，所以重启后它不会再出现。引擎自己保存的原生会话（pi / OMP 的会话文件、DSH 的会话目录）不删除。正在启动的聊天被关闭后，等启动完成再立即释放。

## 9. GPUI 界面（对标原版）

### 9.1 视觉令牌

`app::theme` 模块只做数据，**逐值取自原版**：

- **色板**：`apps/web/src/themes/bundled/{dark,light}.theme.json` 的 `colors` 和 `ansi`。
- **语义角色**：按 `colors.json` 的 `roles` 派生，例如 `text-subtle ← hint`、`container-sidebar-bg ← sidebar`、`border-default ← borderStrong`、`border-muted ← border`。透明度阶梯为 subtle 10%、wash 12%、soft 20%、muted 40%、strong 60%，在 sRGB 空间混合。
- **尺寸**：
  - 顶栏 40；面板表头 28，表头内控件 24；行高 22；面板内边距 8；树缩进 8；聊天边距 8。
  - 间距只用 `0 / 2 / 4 / 6 / 8 / 12 / 16 / 24 / 32 / 40 / 64`。
  - 圆角 xs 2、sm 4（默认）、md 6、lg 8（对话框和用户消息气泡）。
- **字体**：
  - 界面 Geist，代码 JetBrains Mono，品牌 Orbitron。三者均为 OFL 许可，以 TTF 随应用打包（`crates/ui/assets/fonts`，含许可文件），启动时注册。原版 web 用 woff2 可变字体，DirectWrite 不能加载 woff2，所以改用官方发布的静态 TTF（Geist 300/400/500/600、JetBrains Mono 400/500）和 Orbitron 可变 TTF。
  - 字号：ui 14 / light 370；metadata 12；eyebrow 12 medium；caption 10；button 14 / 470；code 13；行高 1.3（代码 1.4）。令牌保留原值；渲染时取最接近的已打包字重（370 → 400、470 → 500），这是与 web 版唯一的已知视觉差异。
- **图标**：使用 gpui-kit 自带的 Lucide 图标（编译期按名选取，不下载）。原版用 Remix Icon，两者形状相近但不完全相同。

**规则**：视图里不出现十六进制颜色或像素常量，一律引用 `thinkrail_ui::theme` 的 `Roles` 和 `metrics` 中的尺寸、排版。主题文件（`crates/ui/themes/*.theme.json`）与原版逐字节相同，角色按原版 `colors.css` 的派生规则计算，并有单测核对。这些映射到 gpui-component 的 `ThemeColor`，并在启动时设置，让组件库的控件也使用相同色板。

### 9.2 布局

以原版 Balanced 布局为基线，参照截图 `workbench-{light,dark}.png`：

- **顶栏**：品牌标识（`text-primary`），然后是三段位置信息：PROJECT / WORKSPACE / BRANCH。每段上方是 eyebrow 标签，下方是带 chevron 的选择器。右侧依次是连接状态（`feedback-success` 圆点加文字）和设置齿轮。
- **左侧活动栏**（约 40 宽）：图标按钮切换左侧面板。选中项是 `control-bg-selected` 底，左缘加 `primary` 色条。
- **Projects 面板**：
  - 表头 28，标题加 `+` 和 `×` 按钮。
  - 树行 22：chevron、文件夹图标、项目名，右侧固定列放 `+` 新建工作区。
  - 工作区行：分支图标、名称，下面一行是 `text-subtle` 的分支名；当前工作区用 `control-bg-selected`，名称用 `primary`。
- **中间聊天列**：
  - 页签条 28，页签带图标和名称，右侧是历史、终端、新建按钮。
  - 工具栏：TODO 列表折叠条加计数。
  - 消息流：用户消息气泡是 `bubble-user-bg`、`bubble-user-border`、圆角 lg；思考和步骤可折叠；工具卡；结尾一行写工具调用数、改动文件数、耗时。
  - 底部输入框：`control-bg`，下方左侧是模型选择器，右侧是发送按钮。
- **右侧**：上面是文件或 Diff 页签区，下面是 Changes 面板（表头加 All changes / vs base 选择器，List / Tree 切换，文件行带 `+N -M`）。
- **底部**：终端面板，背景 `container-terminal-bg`，使用 ANSI 色板。
- **本期不做**：Specs 面板（02 §1 的后续阶段）。不显示占位入口。

### 9.3 GPUI 实践

- **组件**：无状态复用件实现 `RenderOnce` 并 `#[derive(IntoElement)]`，例如 `PanelHeader`、`TreeRow`、`Tab`、`TabStrip`、`IconButton`、`Segment`、`StatusDot`。它们由 props 驱动，不访问服务。
- **视图**：有状态面板是 `Entity<T>`，例如 `ProjectsPanel`、`ChatPanel`、`TranscriptView`、`ChangesPanel`、`FilePanel`、`TerminalPanel`、`SettingsDialog`。只有 `Workbench` 负责组合面板，面板本身不知道自己被放在哪里。
- **状态**：`AppState` 是一个 `Entity`，持有服务 trait 对象和领域投影。视图订阅它的事件（`EventEmitter<AppEvent>` 加 `cx.subscribe`），不直接改其他视图的字段。
- **异步**：服务调用使用 `cx.spawn(async move |this, cx| { let r = cx.background_spawn(...).await; this.update(cx, ...) })`。返回的 `Task` 要存起来或 `detach_and_log_err`，不能直接丢弃。每个请求带代际号，响应回来时代际已经变化就丢弃。
- **事件流**：引擎线程通过 `async_channel` 推送 `SessionEvent`，由 `AppState` 中的一个前台任务消费。每帧只做必要的 `cx.notify()`。消息列表按消息 id 复用卡片 Entity，流式输出时只更新尾部，不重建整个列表。
- **输入**：IME 组合期间按 Enter 不发送；发送快捷键只在非组合状态下生效。所有操作都是 `actions!` 定义的 Action，键盘和鼠标共用同一套 Action。
- **文本**：所有界面文字都是中文，集中放在 `app::strings`，视图里不写字面量。

## 10. 构建与依赖

- 依赖精确锁定（`=x.y.z`）并提交 `Cargo.lock`，`rust-toolchain.toml` 固定 1.99.1。版本见 02 §3。
- **Windows 构建**：
  - 有 MSVC Build Tools 时使用默认链接器。
  - 没有 MSVC 时，用 LLVM（`lld-link`、`clang-cl`）加 `xwin splat` 得到的 SDK，需要设置环境变量 `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER`、`CC`、`CXX`、`AR`、`RC`、`LIB`、`INCLUDE`。这些写在 `docs/BUILD.md`，由开发者自行设置，不提交到 `.cargo/config.toml`。
- debug 构建在移动 target 目录后，必须重新编译 `gpui-fast-windows` 和 `gpui-pre-windows`（01 §4）。
- **lint**：workspace 层统一配置。`unsafe_code` deny；clippy 的 `unwrap_used`、`expect_used`、`print_stderr` 为 warn，`dbg_macro`、`todo`、`unimplemented`、`print_stdout` 为 deny；`clippy.toml` 允许测试中使用 unwrap 和 print。

## 11. 验证要求

- **单元测试**：测可观察的行为和边界，包括路径越界、损坏数据不覆盖、乐观并发冲突、单一 in_progress、锚点漂移和 stale 阻止发送、帧大小与分片重组、交互回复格式、环境变量剔除，以及 revision 单调递增。
- **真实进程测试**（`#[ignore]`，按需运行）：
  - Windows ConPTY 生命周期；
  - pi、OMP、DSH 握手：使用官方 CLI、隔离的配置目录，不带凭据；
  - MCP server 与三个引擎各完成一次真实的工具发现。
- **窗口验证**：打开真实窗口，逐项操作，与原版截图对照。键盘输入只发给已确认获得焦点的测试窗口，不发送全局按键。
- **不能算作完成的证据**：编译通过，或单独一张截图。无法观察的路径必须标为“未验证”。
