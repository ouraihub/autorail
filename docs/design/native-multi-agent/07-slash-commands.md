# 07 斜杠命令：输入框里的 `/` 菜单

状态：**已实现**（2026-11-02）。沿用本设计包的分层、错误与日志规范和事实标记。

目标：聊天输入框以 `/` 开头时，列出当前引擎可用的命令（名称、说明、来源），边输入边过滤，选中后补全；发送时由宿主或引擎执行。行为对标原版 ThinkRail，覆盖 pi、OMP、DSH 三个引擎。

## 1. 原版 ThinkRail 的做法（源码）

| 方面 | 原版行为 | 位置（`thinkrail` 仓库） |
|---|---|---|
| 何时打开 | 整个草稿以 `/` 开头且不含空白 | `apps/web/src/prompt/SlashCommandCompletion.tsx:31-33` |
| 过滤 | 名称不区分大小写子串匹配，保持目录顺序，最多 8 行 | 同上 `:35-45` |
| 按键 | ↑↓ 循环，Enter / Tab 选中，Esc 关闭；输入法组合优先 | 同上 `:69-85`；`apps/web/src/chat/Composer.tsx:421-478` |
| 行内容 | `/名称`、说明、右侧来源：内置行显示 “Pi / 内置”，其余显示 `{source}/{scope}`（如 `extension/temporary`） | `SlashCommandCompletion.tsx:188-211` |
| 选中 | 草稿替换为 `/名称 `，光标移到末尾，菜单随之关闭 | `SlashCommandCompletion.tsx:47-49` |
| 命令来源 | 宿主内置 `/compact`、`/name`、`/model`，加上会话的扩展命令、prompt 模板、技能（`skill:<名称>`）；同名时内置优先 | `apps/web/src/chat/nativeCommands.ts:16-102`；`packages/server/src/agent/agentSessionManager.ts:2759-2775` |
| 执行 | `/compact [指令]` → `session.compact`；`/name <标题>` → `session.rename`；`/model [搜索]` → 打开模型选择器；其他文本原样交给 pi `prompt()`，由 pi 展开或执行 | `ChatView.tsx:551-632`；`Composer.tsx:314-326` |
| 模板页脚 | 宿主模板列表为空时显示“还没有 prompt 模板…”，点击打开“设置 → 模板” | `ChatView.tsx:396-410` |

## 2. 三个引擎的能力

| | pi 1.4.0（源码 + 文档） | OMP 18.15.0 源码（固定版本 18.14.2） | DSH 0.2.2-alpha.2 源码（固定版本 0.2.1-rc.5） |
|---|---|---|---|
| 列命令 | `get_commands` → `{commands:[{name, description?, source: extension\|prompt\|skill, sourceInfo:{scope: user\|project\|temporary, …}}]}`，不含内置命令 | `get_available_commands` → `{commands:[{name, aliases?, description?, input?:{hint}, subcommands?, source: builtin\|skill\|extension\|custom\|mcp_prompt\|file}]}`，含可在文本模式执行的内置命令 | ACP 不提供命令列表，也不发 `available_commands_update` |
| 列表变化 | 无推送 | 推送 `available_commands_update {commands}` | — |
| 发送 `/x` | `prompt` 执行扩展命令、展开模板和 `/skill:x`；扩展命令在本地处理完时回复 `disposition: handled` | `prompt` 执行内置命令、技能、扩展、自定义命令和模板；在本地处理时 `prompt_result.agentInvoked=false`；内置命令的输出以 `command_output {text}` 推送，改名以 `session_info_update {title}` 推送 | 当作普通用户消息；`tool-skill` 插件在每步开始前展开以空白分隔的 `/<技能名>` |
| 压缩 | 专用 RPC `compact {customInstructions?}`，`/compact` 文本不生效 | `/compact [指令]` 文本即可，也有 `compact` RPC | ACP 不支持 |
| 改名 | 专用 RPC `set_session_name {name}`，`get_state.sessionName` 读回 | `/rename <标题>` 文本或 `set_session_name`，`get_state.sessionName` 读回 | ACP 不支持 |
| 切模型 | `set_model` + `set_thinking_level`（已实现） | 同 pi（已实现）；`/model` 文本也可 | `session/set_config_option`（已实现） |

OMP、DSH 的结论读自工作区里的源码树，与应用固定安装的版本不同，标为**源码**。**实测**（2026-11-02，隔离配置目录，不发模型请求）：pi 1.4.0 的 `get_commands` 返回扩展 `/llama`、`/mcp`（`temporary`），用户技能（`user`）和宿主暂存的内置技能（`temporary`），`set_session_name` 能从 `get_state.sessionName` 读回；对空会话调用 `compact` 时 pi 立即拒绝：“Nothing to compact (session too small)”。本机 OMP 18.13.3（低于固定版本 18.14.2）已有 `get_available_commands`，返回 46 个内置命令（含 `/compact`、`/rename`、`/model`），以及技能、扩展、自定义和文件命令；没有一条带 `input.hint`。本机没有安装 DSH，DSH 技能扫描只有单元测试。DSH 技能目录（按优先级）：`<项目根>/.dsh/skills`、`<项目根>/.agents/skills`、`customSkillDirs`（宿主暂存的内置技能）、`<DSH_HOME>/skills`、`~/.agents/skills`。每个目录下识别 `<名称>/SKILL.md` 或 `<名称>.md`，frontmatter 必须有 `name`、`description`，`user-invocable: false` 不能由用户调用；名称须符合 `[a-z0-9]+(-[a-z0-9]+)*`。项目根是离 cwd 最近的含 `.git` 的祖先目录。

## 3. 决定

| 编号 | 决定 |
|---|---|
| C1 | 引擎命令属于会话状态：`SessionSnapshot` 增加 `commands: Vec<SlashCommand>` 和 `session_name: Option<String>`，由各适配器填写，界面只读快照。 |
| C2 | pi：握手时调用 `get_commands`；`/compact`、`/name` 由宿主拦截，分别调用 `compact`、`set_session_name` RPC。`Capabilities` 增加 `compact`、`rename`，只有 pi 为 `true`。 |
| C3 | OMP：握手时调用 `get_available_commands`，并处理 `available_commands_update`；它的内置命令（`/compact`、`/rename` 等）由 OMP 自己执行，宿主不拦截。另外处理 `command_output`（显示为系统消息）和 `session_info_update`（更新会话名）。 |
| C4 | DSH：宿主扫描第 2 节列出的技能目录，生成 `/<名称>` 命令列表；执行时原样发给 DSH。DSH 没有压缩和改名。 |
| C5 | `/model [搜索词…] [力度]` 是宿主内置命令，凡是支持切模型的引擎都提供，对候选模型（提供方名、模型名、id、路由）做多词匹配：匹配到 1 个就切换；匹配到多个就列出候选并报错；不带参数时提示用法。原生版没有原版那种可搜索的模型选择器，所以不弹窗，直接在命令里给出结果。 |
| C6 | 合并顺序：宿主内置（`/compact`、`/name` 按能力，`/model`）→ 引擎命令；引擎命令与宿主内置同名时不显示（宿主优先，同原版）。 |
| C7 | 来源标签：内置命令（宿主的和 OMP 自己的）显示 `{引擎} / 内置`，与原版的 “Pi / 内置” 一致；其余显示 `{source}/{scope}`，没有 scope 时只显示 source。 |
| C8 | 聊天未连接引擎时（应用重启后尚未发送消息），菜单只显示宿主内置命令，能力取自 `AgentRuntime::capabilities()`；执行内置命令需要引擎时，先启动引擎并保留草稿，启动后再按一次 Enter。 |
| C9 | 不做模板页脚：原生版没有“设置 → 模板”页面，pi 的 prompt 模板照常出现在列表里（来源为 `prompt`）。 |
| C10 | 标签页标题优先用 `session_name`，其次用第一条用户消息，最后用“聊天 N”。 |
| C11 | 菜单列出全部匹配项，可以滚动，一屏显示 8 行；↑↓ 移动时选中行保持可见，内容一变就回到顶部。原版只取前 8 个匹配（`MAX_MATCHES`），技能一多就看不到后面的命令，用户决定改成可滚动（2026-11-02）。 |
| C12 | 引擎拒绝压缩（例如会话太短，“Nothing to compact”）不算应用错误：作为一条“压缩失败。…”系统消息写进对话，聊天保持空闲，草稿照常清空，同原版的“压缩失败”轮次。超时、进程退出等仍按错误报告。 |
| C13 | `/model` 只带力度、而当前模型不在候选列表里时，报错“当前模型不在可选列表中，无法只改力度”；聊天不空闲时 `/model` 与头部下拉一样不执行，草稿保留。 |

## 4. 契约变化（设计）

```rust
// core
pub enum CommandSource { Builtin, Extension, Prompt, Skill, Custom, McpPrompt, File }
pub enum CommandScope { User, Project, Temporary }
pub struct SlashCommand {
    pub name: String,                 // 不带 `/`
    pub description: Option<String>,
    pub hint: Option<String>,         // OMP `input.hint`
    pub source: CommandSource,
    pub scope: Option<CommandScope>,  // pi `sourceInfo.scope`；OMP、DSH 没有
}
// SessionSnapshot: + commands, + session_name
// Capabilities:    + compact, + rename
// AgentSession:
fn compact(&self, instructions: Option<&str>) -> Result<()>; // 默认返回 INVALID “{引擎} 不支持 /compact”
fn rename(&self, name: &str) -> Result<()>;                  // 默认返回 INVALID “{引擎} 不支持 /name”
```

pi 的 `compact` 等到压缩完成：先检查空闲，置为运行中，超时 10 分钟（摘要要调用一次模型，30 秒的通用超时不够），完成后追加一条系统消息“上下文已压缩（约 X → Y tokens）”，并恢复空闲；引擎拒绝时按 C12 追加“压缩失败。…”。`rename` 成功后更新快照里的 `session_name`。`core::Error::detail()` 返回不带错误类型前缀的说明，供这条消息使用。

## 5. 模块落点

| 位置 | 内容 |
|---|---|
| `crates/core/src/commands.rs`（新） | `SlashCommand` 等类型 |
| `crates/agents/src/rpc/commands.rs`（新） | 解析 pi `get_commands` 和 OMP `get_available_commands`、`available_commands_update` 的载荷 |
| `crates/agents/src/projection/bound.rs`（新） | `projection.rs` 已超过 400 行：把消息大小上限、截断和工具结果合并从中拆出；命令、会话名的写入方法留在 `projection.rs`（只在变化时发布） |
| `crates/agents/src/pi.rs`、`omp.rs` | 握手取命令；pi 实现 `compact`、`rename`；OMP 处理三种新帧 |
| `crates/agents/src/dsh/skills.rs`（新） | 扫描 DSH 技能目录 |
| `crates/agents/src/smoke/{commands,skills,model}.rs`（新） | `smoke.rs` 原有 645 行：按主题拆出；新增命令列表、改名和空会话压缩的冒烟 |
| `crates/app/src/panels/chat/commands.rs`（新） | 纯逻辑：识别查询、合并与过滤、解析要发送的内置命令、`/model` 匹配；测试在 `commands/tests.rs` |
| `crates/app/src/panels/chat/slash.rs`（新） | 菜单状态与渲染（可滚动，C11）、按键拦截（在捕获阶段拦下 ↑↓、Enter、Tab、Esc）、选中补全；无界面窗口测试在 `slash/tests.rs` |
| `crates/app/src/panels/chat/session.rs` | `send` 先处理宿主内置命令，再走普通 prompt；`routing.rs` 的 `set_model` / `set_effort` 改为返回是否已发出请求 |

## 6. 验证

- 单元测试：pi / OMP 载荷解析、DSH 技能扫描（优先级、`user-invocable: false`、非法名称）、查询识别、合并与过滤（宿主优先、不截断）、内置命令解析、`/model` 匹配（0 个、1 个、多个、带力度）。
- 无界面窗口测试：在输入框里输入 `/` 后菜单出现；↓ 加 Enter 把草稿补全成 `/名称 `，且不会把消息发出去；Esc 关闭菜单。
- 真实引擎冒烟（`#[ignore]`）：pi `get_commands`、`set_session_name` 读回、空会话 `compact` 变成“压缩失败。…”消息；OMP `get_available_commands`（用本机安装的版本，并记录版本号）。
- 真实窗口（**实测**，2026-11-02，pi 1.4.0）：未连接引擎时只显示 3 个宿主内置命令；`/name 斜杠测试` 第一次 Enter 启动引擎，第二次改名，标签页标题随之变化；菜单列出全部命令（扩展、用户技能、内置技能），↑ 从第一行循环到最后一行时列表跟着滚动，鼠标滚轮也能滚动；空会话 `/compact` 在对话里显示“压缩失败。Agent 报告：Nothing to compact (session too small)”，聊天保持空闲，没有报错栏。压缩成功的情况要调用一次模型，没有在窗口里跑。
