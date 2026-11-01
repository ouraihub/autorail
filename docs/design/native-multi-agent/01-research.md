# 01 调研与需求分析

本文件只记录事实与证据，不规定接口；接口见 [详细设计](04-detail.md)。

## 1. 结论

可以用纯 Rust 实现：业务（项目、worktree、Git、文件、终端、审查、计划、Agent 档案）由 Rust 服务承担；三个引擎通过各自的**官方进程协议**接入，不嵌入任何 JS SDK。应用自身不需要 Bun 或 Node；引擎及其 Node / Bun 用户已装的直接用，没装时由“编码搭档”页下载（U06）。

代价：进程协议暴露的能力少于进程内 SDK。宿主工具（TODO / 审查）不能在进程内注册，需要通过 MCP 桥提供（04 §6）；各引擎能力差异要如实反映在 `Capabilities` 中。

## 2. 原版 ThinkRail 行为基线

原版仓库 `E:/workspace/notepad-dev/thinkrail` 只作为行为和视觉参考，不复制其 TS 实现。需要保留的行为：

- 并发聊天、流式文本与思考、工具卡、运行中错误与取消、历史恢复；文本 delta 追加，工具 partial 结果替换。
- 一轮完成以引擎的 settle 信号为准（pi `agent_settled`、OMP `prompt_result`/`session_settled`、DSH `session/prompt` 返回），`agent_end` 不代表完成。
- 引擎提问与权限必须由用户显式回答；关闭对话框即取消，不能自动批准。
- TODO：保留用户项、同时只有一个 in_progress、done 项不可删除。审查：锚点漂移检测、stale 评论阻止发送。
- 项目 / worktree：默认工作区不可删除；外部 worktree 不能当作自建 worktree 删除。
- 终端：tabKey 持久、PTY 不持久；关闭视图与关闭 shell 是两件事；attach 先回放再接实时输出。

## 3. Agent 接入事实

| 引擎 | 版本 | 接入方式 | 证据 |
|---|---|---|---|
| pi | 1.4.0（npm 最新） | `node dist/bundle/cli.js --mode rpc`，stdin/stdout JSONL（非 JSON-RPC） | **实测**：在隔离配置目录、无凭据的条件下，`get_state` 返回 `sessionFile` / `sessionId` |
| OMP | 18.14.1（npm 最新） | `bun src/cli.ts --mode rpc-ui`；需 Bun ≥ 1.4.4 | **版本元数据已核实**：npm `@oh-my-pi/pi-coding-agent` 最新版为 18.14.1，要求 Bun ≥ 1.4.4；本适配器仍需用隔离配置目录和官方 CLI 做真实握手复核 |
| DSH | 0.2.1-rc.5（npm `latest`，没有正式版） | `node lib/bin.js --profile acp`，JSON-RPC ACP；需 Node ≥ 24.9.0 | **实测**：临时目录 npm 安装，Node v24.21.0，被阻止的 5 个安装脚本都不需要启用。`initialize` 返回 `protocolVersion: 1`，`mcpCapabilities: {http:true}`，`sessionCapabilities` 含 `close`、`list`、`resume`。`session/new` 不需要模型和凭据，返回 `sessionId` 和 `configOptions`。`session/resume` 在新进程中成功 |

协议要点（**源码**，实现前须用固定版本复核）：

- **pi**：`prompt` 的 response 只是受理确认，完成要看 `agent_settled`。交互请求是 `extension_ui_request`（select / confirm / input / editor）。关闭 stdin 即退出。pi 不会在每次工具调用前请求审批，不能宣称支持逐次工具审批。
- **OMP**：启动时先发 `ready` 帧。有精确的终止帧 `prompt_result` 和 `session_settled`。问卷需要先 `set_ask_dialog` 再走 `ask` 请求。v1 单帧上限 1 MiB；v2 的 `rpc_chunk` 用 base64 分片，重组上限 64 MiB。审批模式必须是 `always-ask`。
- **OMP 特有（实测）**：没有可用的默认模型时，进程直接退出，stderr 是 “No default model selected”（pi 没有这个限制）。用户的 OMP 配置目录中必须有可用模型（如 `models.yml`），适配器要把这条 stderr 原样作为启动错误显示。启动后 OMP 会主动推送非交互的 `extension_ui_request`（例如 `setWidget`），适配器只把 select / confirm / input / editor / ask 当作交互处理，其余忽略。
- **OMP 停止后的上下文（实测 18.13.3）**：没有正文、也没有工具调用的 `aborted` 回复，会被 `transformMessages` 从发给模型的上下文中删掉，但它对应的请求还在。下一轮模型看到的是连续几条用户消息，会去补做用户已经停止的请求。适配器在 `stop` 之后的下一次 `prompt` 前，先用 `get_entries` 找到末尾连续被停止的请求中最早的一条，再 `navigate_tree` 到这条请求（`summarize: false`），把叶子退回它的父节点。新请求挂在新分支上，原分支仍保留在会话文件里。回复已经输出正文或调用过工具时不撤回，这时 OMP 会把它发给模型。代价：重启后按活动分支回放，被撤回的请求不再出现在对话记录里。
- **DSH**：方法有 `initialize`、`session/new`、`session/resume`、`session/prompt`、`session/cancel`（notification）、`session/close`、`session/list`；`session/load` 返回 -32601。消息分块是已提交内容，不是 token delta。权限请求走 `session/request_permission`。没有历史回放，也没有问卷。
- **DSH 特有（实测）**：
  - launcher 参数（`--profile`、`--patch`）必须放在最前面。遇到第一个不认识的 token 后，后续参数都交给 app 处理，所以不能追加在末尾。
  - `session/resume` 的返回不含 `sessionId`，locator 就是请求时传入的 id。
  - 模型的 `currentValue` 是 JSON 编码的 `[provider, model]`，显示名要到 `options` 里按 value 查。推理档位项的 `category` 是 `thought_level`。
  - ACP 新会话的初始模型取自 `acp` 入口自己的 `provider` / `model`（`dsh-acp-app` 固定为 `deepseek-official/deepseek-v4-flash`），**不读** `agent-default-model`。要用中转模型，配置目录的 `profiles/acp/cordis.patch.yml` 除了在 `llm-pi-ai` 中声明 provider，还要加 `- id: acp` / `config: { provider, model }` 覆盖。
  - DSH 会加载**进程 cwd** 下的 `.env` 作为凭据来源之一，因此子进程 cwd 设为 `DSH_HOME`，工作区只通过 `session/new.cwd` 传入。
  - DSH 不校验 `cwd` 是否存在，宿主要自己校验。
  - `agentInfo.version` 固定为 `0.0.1`，不能用来判断版本。
  - 内置 `todo_write` 可以关闭：用 `--patch <file>` 传入 `- id: tool-todo\n  disabled: true`。

宿主工具（U08）的接入方式见 04 §6。

## 4. Windows 构建与运行事实（实测）

- 工具链 Rust 1.99.1（最新稳定版），依赖全部来自 crates.io 正式版，没有 Git 依赖。
- 本机没有系统 MSVC：PATH 上的 `link.exe` 实际来自 Git for Windows。构建统一用便携 MSVC 14.44 + Windows SDK 26100（`scripts/msvc/cargo-msvc.bat`，取自 `@ouraihub/desktop-kit`，与 AutoNote 共用），2026-10-31 实测全量构建通过。此前用过 LLVM + xwin，但它没有 SDK 的 `rc.exe`，`turso_sdk_kit` 的构建脚本因此失败，已弃用。
- GPUI Fast 0.1.12 的 debug 构建在运行时从编译期写入的绝对路径读取 HLSL，移动 target 目录后会报 `Creating DirectX render pipelines: 0x80070003`。对 `gpui-fast-windows` 和 `gpui-pre-windows` 执行 `cargo clean -p` 后恢复正常；release 构建会把 shader 编译进二进制。
- portable-pty 0.9.0 的 `WinChildKiller::kill` 把 `TerminateProcess` 的返回值判反了：成功时报 `os error 0`。实现以等待线程观察到进程实际退出为准。
- Git 不接受 `\\?\` verbatim 路径作为 worktree 路径：传给 Git 的路径要去掉这个前缀，持久化的仍是规范化路径。
- `gpui_kit::*` 通配导入会遮蔽 `#[test]`。这些模块的测试要显式写 `#[::core::prelude::v1::test]`，或者不要通配导入。
- 运行时：Node v24.21.0（已装）、Node 26.13.0 / Bun 1.4.4（最新正式版，下载并校验到临时目录，未安装进系统）。

## 5. 风险登记

| 编号 | 风险 | 处置 |
|---|---|---|
| Q01 | DSH 只有 alpha，没有正式版 | 如实标注；锁定具体版本，升级时重测 |
| Q02 | 进程协议能力少于 SDK | 按引擎声明 `Capabilities`；缺的能力不伪造；宿主工具改走 MCP 桥 |
| Q03 | 引擎协议漂移 | 锁定版本；每个引擎有 ignored 的真实握手测试，升级时运行 |
| Q04 | GPUI Fast / gpui-kit 尚在 0.x | 精确锁定版本；升级前先跑窗口冒烟测试 |
| Q05 | 不同档案之间串用凭据 | 子进程启动时剔除继承环境中的凭据和引擎配置变量，只注入档案显式引用的变量 |
| Q06 | 本机缺少 MSVC 环境 | 统一用便携 MSVC，`scripts/msvc/setup-msvc.bat` 安装、`cargo-msvc.bat` 执行（`docs/BUILD.md`） |
