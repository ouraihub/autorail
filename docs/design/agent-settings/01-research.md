# Agent 设置调研：模型、提供方、MCP 服务器

状态：调研，2026-10-31。只记录事实和待决问题，不含方案。事实标记沿用 [多 Agent 设计包](../native-multi-agent/README.md#事实标记)：**实测**、**源码**、`[INFERENCE]`。

目标：在原生版设置对话框的 AGENT 分组下实现“模型”“提供方”“MCP 服务器”三页，界面对标原版 ThinkRail（U04），并让 pi、OMP、DSH 三个引擎都能使用用户配置的中转模型。

依据：

- 原版：`E:/workspace/notepad-dev/thinkrail`，`main` 分支 `ec2f2db5`（2026-10-31 拉取）。
- 引擎：pi 1.4.0（`node_modules/.bun/@earendil-works+pi-*`）、OMP 18.14.1（npm 最新，要求 Bun ≥ 1.4.4）、DSH 0.2.1-rc.5（npm 发布包 `@deepseek-ai/*` 的 README 与 `lib/index.js`）。
- 参考实现：Cherry Studio，`E:/docs/AI申请/AI安装配置教程/ai/cherry-studio`（第 4 节）。
- 原生版：`thinkrail-native` 提交 `ff1e9e3`。

## 1. 原版设置对话框（源码）

### 1.1 外壳

文件：`apps/web/src/panels/SettingsDialog.tsx`、`settingsNav.ts`。尺寸单位为 1px（`apps/web/src/styles/SPACING.md`）。

- 对话框：高 `80vh`（最大 `85vh`），最大宽 `52rem`，无内边距。
- 头部：`px-16 py-8`，下边框；标题“设置”，右侧关闭按钮。
- 左侧导航：宽 192，内边距 12，背景 `container-elevated-bg`，右边框。
  - 分组标题高 28，`label-pill` 样式，`text-subtle`；分组间距 12，项间距 2。
  - 导航项高 28、`px-8`、14px 图标、图标与文字间距 4；选中项 `bg-primary-subtle text-primary`。
  - 窄屏时导航变为横向条。
- 分组与项：
  - AGENT：模型（`RiBrainLine`）、提供方（`RiKeyLine`）、MCP 服务器（`RiPlugLine`）、Prompt 模板、环境。
  - 界面：会话、外观、语言、布局、终端。
  - 系统：工作区、集成、扩展、通知、隐私、更新。
  - 模型页和 MCP 页按协议版本显示（`DEFAULT_MODEL_PROTOCOL_VERSION` 72、`MCP_PROTOCOL_VERSION` 83）。
- 底部：“分享反馈”按钮固定在导航底部，`border-primary-soft`、`text-primary`、`px-12 py-8`。
- 内容区：`flex-1 overflow-y-auto p-16`。焦点在草稿输入框中时 Esc 不关闭对话框。

### 1.2 共用布局

- `SettingsGroup`：分组标题（pill 样式）+ 可选说明，底部 12；组内最后一行无下边框。
- `SettingsPageContainer`：组与组之间 `mt-24 border-t pt-24`。
- `SettingsRow`：`py-12`、下边框；左侧标签和说明（最小宽 160），右侧控件。

### 1.3 模型页

文件：`panels/ModelsSettings.tsx`、`VisibleModelsSettings.tsx`、`ModelContextSettings.tsx`；力度控件 `chat/ThinkingSelector.tsx`、`chat/modelPicker.ts`。

| 分组 | 行 | 控件与行为 | 存储 |
|---|---|---|---|
| 默认模型 | 会话模型 | 模型下拉（占位“首个可用”，下拉底部有刷新）+ 力度下拉 | `~/.thinkrail/config.json` 的 `defaultModel`、`defaultEffort` |
| 默认模型 | 可见模型 | 行内展开（右箭头 ↔ 下箭头）；搜索框；每个模型一行带开关；说明“新模型会自动显示。”或“已隐藏 N 个模型。新模型会自动显示。” | 客户端本地，只记录被隐藏的模型 |
| 评审 | 评审模型 | 模型下拉，默认项“{{name}}（默认）”；选默认项写 `reviewModel:null`；力度用评审模型的档位 | `config.json` 的 `reviewModel`、`reviewEffort` |
| 评审 | Agent 触发的评审 | 开关，“允许 agent 评审已完成的工作。” | `config.json` 的 `agentReviewEnabled` |
| 评审 | 自动修复循环 | 开关，“把要求修改的反馈一次性发回给 agent。” | `config.json` 的 `reviewAutoFix` |
| GPT 上下文窗口 | 全局 / 每个模型 | 只列 OpenAI Responses 类模型，否则显示“没有符合条件的模型” | pi `models.json` 的 `modelOverrides.<id>.contextWindow` |

- 力度档位取自模型自身的 `thinkingLevels`，为空时控件禁用。文案：关闭、最低、低、中、高、极高、最高（off、minimal、low、medium、high、xhigh、max）。
- 未设置时显示的力度：模型解析出的默认值 → `defaultEffort` → `medium`。
- 加载中隐藏该行；加载失败显示“无法加载您的默认模型。”和“重试”。保存失败用 toast。
- 可见模型行：提供方图标、名称（隐藏时变暗）、`{{provider}} · {{window}}`，推理模型追加“· 推理”。空状态“没有匹配的模型。”“没有可用模型。”
- 依赖进程内 pi：默认模型按 pi 的实时模型表解析并裁剪力度；上下文窗口覆盖只对 pi 的 Responses 类模型有效。评审模型和两个开关服务于原版宿主的 `request_review` 工具。

### 1.4 提供方页

文件：`panels/ProvidersSettings.tsx`、`ProviderRow.tsx`、`useProviderAuth.tsx`、`auth/LoginDialog.tsx`、`CustomEndpointsSettings.tsx`、`customEndpointDraft.ts`；服务端 `auth/providerStatus.ts`、`providerLogin.ts`、`agent/customEndpoints.ts`、`agent/modelsFile.ts`。

从上到下：

1. 头部说明“至少连接一个提供方才能运行 agent，可以是订阅或 API 密钥。”，右侧“刷新”按钮。加载中显示 3 行骨架；失败显示“无法从 host 读取提供方状态，请尝试刷新。”
2. **已连接**：名称、类型（订阅 / API 密钥 / 环境变量 key / JetBrains AI / 已配置）和可选说明。可退出时显示“退出登录”，否则显示锁形“托管中”。
3. **JetBrains AI 卡片**：依赖 JetBrains central CLI。
4. **自定义服务器**（#1084，协议版本 101）：说明“公司大模型网关或本机上的模型——任何兼容 OpenAI 的服务器。”每行显示名称、`{{url}} · {{count}} 个模型`、“编辑”和删除按钮；底部“添加服务器”。
5. **订阅**：支持 OAuth 的未配置提供方（Claude、ChatGPT、Copilot 套餐），按钮“登录”，可用 API 密钥的另有“API 密钥”。
6. **可用（N）**：只支持 API 密钥的提供方，先显示 8 个，其余用“再显示 N 个”展开。
7. 底部提示“应用外还配置了 N 个（环境变量或 models.json）：…”。

自定义服务器对话框（最大宽 36rem）：

- **名称**。
- **地址**：代码字体，占位 `https://llm.example.com/v1`；下方本地预设 Ollama `:11434/v1`、LM Studio `:1234/v1`、vLLM `:8000/v1`、llama.cpp `:8080/v1`。编辑时不能改 origin（“服务器地址不可更改。要使用其他服务器，请将其添加为新服务器。”）。
- **API 密钥**：密码框，占位“可选 · 支持 $ENV_VAR”，已保存时“已保存 · 输入以替换”，可切换“删除密钥 / 保留密钥”。密钥只留在宿主，不回传界面。
- **模型**：“查找模型”请求服务器的模型列表（编辑时打开即自动查找），每行显示模型 id 和“上下文 · 思考 · 图像 · 工具”，用开关选择；首次查找全部默认关闭，之后保留原有选择。也可手动输入模型 id 添加。
- 状态行：“找到 N 个模型”；失败分类为地址无效、密钥被拒、超时、无法连接、不是 OpenAI 兼容响应、HTTP 状态码、没有模型。
- “保存”的条件：名称非空、地址非空、至少选一个模型、origin 未变。删除前确认：“其模型将从模型选择器中移除。使用过它们的聊天会保留历史记录。”

存储：写入 pi `<agentDir>/models.json` 的 `providers["custom-<slug>-<6hex>"]`，字段 `{name, baseUrl, api:"openai-completions", apiKey, compat, models[]}`；无密钥时写占位值 `thinkrail-no-key`。保存后宿主调用 `runtime.refresh` 并广播 `provider.changed`，模型随即出现在选择器中，pi CLI 也能看到。

依赖进程内 pi：已连接列表、能否 OAuth 或 API 密钥、认证类型都来自 pi 运行时（`getProvider`、`listCredentials`、`getProviderAuthStatus`）；登录走 `runtime.login` 的交互回调。

### 1.5 MCP 服务器页

文件：`panels/McpSettings.tsx`、`McpServerDialogs.tsx`、`mcpEntries.ts`、`mcpPresets.ts`、`mcpServerView.ts`、`lib/mcpState.ts`；服务端 `host/mcp.ts`、`agent/mcp/*`（移植自 pi 的 MCP 加载、校验与写入，本身不建立连接）。

- 依赖当前工作区：没有工作区时显示“打开工作区以管理其 MCP 服务器。”
- 头部：标题、说明（用户级覆盖所有项目，项目级写在 `.pi/mcp.json`），列表非空时显示“添加服务器”。
- 横幅：始终显示“修改数据前会在聊天中确认”的说明；另有“由其他程序管理”（页面只读）、项目未受信任（隐藏项目级条目）、配置文件出错、保存反馈。
- 空状态：“还没有 MCP 服务器。”、内嵌预设列表、“添加服务器”“导入 mcpServers JSON”，以及“服务器在运行 ThinkRail 的 host 上运行，并继承其环境。”
- 每个服务器一张卡片：
  - 名称、范围（用户 / 项目）、传输方式（stdio / HTTP）、暴露级别（延迟 / 直接 / 隐藏）。
  - 状态行：已连接 · N 个工具、正在启动、需要登录、失败、未连接、已禁用、等待批准、配置无效、等待重新加载等；已连接但 0 个工具显示为警告。
  - 按状态给出主操作（登录、重新连接、评审并批准、编辑、立即重新加载、测试连接）、启用开关（用户级为“在此项目中启用/停用”）和 ⋮ 菜单（登录 / 退出登录、测试连接、编辑、共享到仓库、详情、删除）。
  - “此项目中始终允许”的工具列表，可逐个撤销。
- 添加对话框：
  - 预设：Context7、DeepWiki、Linear（只读）、Sentry、Playwright（`npx -y @playwright/mcp@0.0.83`，默认停用）；已存在的显示“已添加”。另有 GitHub（只读）配方卡。
  - 两种方式：表单 / 粘贴 JSON；两种范围：用户（所有项目）/ 项目（`.pi/mcp.json`，需信任项目）。
  - 表单字段：名称（`^[A-Za-z0-9_-]+$`，同范围内唯一）、传输方式（stdio 命令 / HTTP URL）、HTTP 的 URL（只能 http(s)，URL 不能带凭据，`/sse` 给警告）、stdio 的命令和参数（每行一个）、环境变量 / 请求头键值行、暴露级别（默认延迟）、描述（可选）。
  - 环境变量和请求头的值只能是 `$VAR`、`${VAR}` 或 `!command`，不接受明文。
  - 会在宿主上执行命令时，必须经过“确认将运行的内容”这一步。
- 删除确认：“从 {{source}} 中删除此条目。会话下次空闲重载后不再使用它。”
- 存储：用户级 `<agentDir>/mcp.json`，项目级 `<worktree>/.pi/mcp.json`，都是 `mcpServers` 映射；写入是原子的，保留未知键和缩进。项目审批（按指纹）、项目级覆盖、工具授权记在 ThinkRail 的 `projects.json`，不写进仓库。写操作带 `expectedFingerprint` 做乐观并发。
- 依赖进程内 pi：运行状态、工具数、日志来自宿主内运行的 pi 会话；重新连接和重载按聊天进行；OAuth 走 pi 的 `/mcp login`；暴露级别和“由其他程序管理”是 pi 内置 MCP 的概念。

## 2. 原生版现状（源码）

- 设置对话框：`crates/app/src/panels/settings.rs`，宽 760，只有一页可滚动的 Agent 档案列表（新建、编辑、删除、设为默认），没有左侧导航。文案在 `crates/app/src/strings/settings.rs`。
- 可复用组件：`crates/ui/src/components.rs`（`IconButton`、`Segment`、`Status`、`StatusDot`、`ActivityItem` 等）；gpui-component 0.8.1 提供 `Select`、`Switch`、`SidebarMenu`、`setting::Settings` 和密码输入；`theme_map.rs` 已把 ThinkRail 配色映射到这些控件。
- 档案（`crates/core/src/profile.rs`）：`ProfileInput { backend_id, label, enabled, config_root, launch, env_refs }`。档案不保存密钥：`env_refs` 只记录变量名，`validate.rs` 拒绝明文值。每个档案的 `config_root` 是引擎自己的配置目录（pi、OMP 用 `PI_CODING_AGENT_DIR`，DSH 用 `DSH_HOME`），启动时剔除继承的凭据类变量。
- 模型：`SessionSnapshot.model` 只用于聊天头部显示。`AgentSession` 只有 `snapshot / prompt / stop / respond / dispose`，`StartSession` 不带模型；没有适配器调用列模型或切换模型的接口。
- MCP：引擎只收到宿主自己的 `thinkrail` 工具服务器（pi 用 `-e` 扩展，DSH 用 `session/new.mcpServers`，OMP 用 `set_host_tools`）；用户的 MCP 服务器无处配置，也不会传给引擎。
- 宿主目前不写引擎配置目录中的任何文件；只在数据目录写 DSH 的 `engines/dsh-host.patch.yml`，通过 `--patch` 传入。
- 原生版没有评审功能（无 `request_review` 等工具）。

## 3. 三个引擎的配置能力（源码）

### 3.1 提供方与模型

| | pi 1.4.0 | OMP 18.14.1 | DSH 0.2.1-rc.5（ACP） |
|---|---|---|---|
| 文件 | `<dir>/models.json` | `<dir>/models.yml`（旧 `models.json` 会被迁移） | `profiles/acp/cordis.patch.yml` 中 `id: llm-pi-ai` 的 `config.providers` |
| 提供方字段 | `name, baseUrl, api, apiKey, headers, authHeader, compat, models[], modelOverrides` | `baseUrl, api, apiKey, headers, authHeader, auth, discovery, compat, models[], modelOverrides` | `displayName, api, baseURL, apiKeyEnv, headers, compat, models[], modelOverrides` |
| 模型字段 | `id, name, reasoning`(布尔)`, input, cost, contextWindow, maxTokens` | 同 pi，另有 `maxContextWindow` | `id, name, contextWindow, maxTokens, input, reasoningEfforts` |
| 协议 | 很多 | 13 种 | 只有 `openai-completions`、`openai-responses`、`anthropic-messages` |
| 按变量名引用密钥 | `apiKey: "$NAME"`；不带 `$` 视为明文 | `apiKey: NAME`；变量未设置时把名字本身当密钥发出 | `apiKeyEnv: NAME`；依次查 `.credentials.yaml` 的 `refs`、启动环境、cwd 的 `.env`、`$DSH_HOME/.env` |
| 自带密钥存储 | `auth.json` | SQLite 认证库 | `.credentials.yaml` |
| 不写文件的注入 | `-e` 扩展调用 `registerProvider` | `-e` 扩展 `registerProvider`（RPC 模式下未验证） | `--patch` 覆盖层（原生版已在用） |

要点：

- 三者共同支持的协议是 `anthropic-messages`、`openai-completions`、`openai-responses`。用户的 easyclaude 中转是 `anthropic-messages`；原版自定义服务器只支持 `openai-completions`。
- pi：自定义模型要等密钥能解析出来才算“可用”，才会出现在 `get_available_models` 里。
- OMP：自定义的 `anthropic-messages` 提供方要显式写 `auth: apiKey`，否则按 OAuth 方式组装请求。
- DSH：路由名与内置 pi-ai 提供方同名（如 `anthropic`）时继承其地址和模型表；`models` 列表会**替换**整个模型表；手写的模型没有 `reasoningEfforts` 就不推理。

### 3.2 默认模型与力度

| | pi | OMP | DSH（ACP） |
|---|---|---|---|
| 默认模型 | `settings.json` 的 `defaultProvider`、`defaultModel` | `config.yml` 的 `modelRoles.default: "提供方/模型:档位"` | `acp` 入口的 `config.provider`、`config.model`；**不读** `agent-default-model`（**实测**，见多 Agent 设计包 01） |
| 默认力度 | `defaultThinkingLevel`（默认 `medium`） | `defaultThinkingLevel`（默认 `high`，多一个 `auto`） | 无配置项，新会话用提供方默认 |
| 命令行覆盖 | `--model p/id:档位`、`--thinking`、`--api-key` | `--model`、`--thinking`、`--config <yml>` 覆盖层 | `--patch <yml>` |
| 恢复会话 | 恢复会话里保存的模型和档位 | 同 pi | 沿用会话最后一次的路由和档位 |

### 3.3 运行时切换模型

| | pi / OMP（同一套 RPC） | DSH（ACP） |
|---|---|---|
| 列出模型 | `get_available_models` → `{models}`（OMP 会等后台发现完成） | `session/new` 返回的 `configOptions` 中 `id: "model"` 的分组选项 |
| 切换模型 | `set_model {provider, modelId}` | `session/set_config_option {sessionId, configId: "model", value: "[\"提供方\",\"模型\"]"}` → 新的 `configOptions` |
| 力度 | `get_available_thinking_levels`（非推理模型返回 `["off"]`）、`set_thinking_level` | `configOptions` 中 `category: "thought_level"` 的选项，同样用 `set_config_option` |
| 是否写回配置 | 否，只对当前会话生效（RPC 不传 `persist`） | 否 |

### 3.4 MCP 服务器

| | pi | OMP | DSH（ACP） |
|---|---|---|---|
| 配置文件 | `<dir>/mcp.json`、受信任项目的 `.pi/mcp.json` | `<dir>/mcp.json`、`.omp/mcp.json`，以及项目中的 `.claude/`、`.cursor/`、`.vscode/` 等外部配置 | `cordis.patch.yml` 中 `@deepseek-ai/dsh-mcp-client` 条目 |
| 结构 | `mcpServers: {名称: {command, args, env, cwd} 或 {url, headers, oauth}}`，另有 `enabled`、`timeout`（秒）、`exposure` | 同结构，另有 `disabledServers`、`enabledServers`；`timeout` 单位是毫秒 | — |
| 值引用 | `env`、`headers` 支持 `${VAR}` 和 `!cmd` | `${VAR}`、`${VAR:-默认}`、`!cmd`，整值为变量名时按变量解析 | — |
| 启动时注入 | `-e` 扩展调用 `registerMcpServer`，只对本会话有效；同名时 `mcp.json` 优先（原生版已用这种方式注入宿主工具） | RPC 模式**没有**注入接口；只有 ACP 模式接受 `mcpServers` | `session/new`、`session/resume` 的 `mcpServers`（`env`、`headers` 为列表形式） |
| 传输方式 | stdio、streamable HTTP（拒绝 `sse`） | stdio、http、sse | stdio、http |

要点：

- 三者共同支持：名称 `[A-Za-z0-9_-]`，stdio `{command, args, env}`，HTTP `{url, headers}`。
- OMP 默认会读取项目中 `.claude`、`.cursor`、`.vscode` 等外部 MCP 配置，即使配置目录是隔离的；可用 `mcp.enableProjectConfig: false` 或 `disabledProviders` 关闭。
- DSH 的 MCP 服务器启动失败会导致整个会话创建失败；pi 和 OMP 会继续。

### 3.5 配置何时生效

- pi：`models.json` 只在启动和注册提供方时读取；RPC 没有重载命令。
- OMP：`config.yml` 有文件监听；`models.yml` 按修改时间刷新，但 RPC 模式下没有找到触发它的调用 `[INFERENCE]`；`mcp.json` 要 `/mcp reload`。
- DSH：监听 `.credentials.yaml`；其余配置在启动时读取。
- 推论：改了提供方或 MCP 配置后，宿主应重启相应的引擎进程；正在运行的聊天要在引擎重启后才生效。

## 4. 参考实现：Cherry Studio（源码）

项目：`E:/docs/AI申请/AI安装配置教程/ai/cherry-studio`（Electron）。Cherry 有自己的“模型服务”设置，pi 和 DSH 都能用里面配置的服务商。两者的做法不同。只读了代码，没有运行。

### 4.1 pi：进程内注入，不写任何文件

pi（`@earendil-works/pi-coding-agent` 1.4.0）在 Cherry 主进程内运行，代码在 `src/main/ai/runtime/pi/`、`src/main/ai/agents/.../modelInjection.ts`。

- 每个会话把 Cherry 的“服务商 + 模型”翻译成一个 pi 提供方配置：`{name, baseUrl, apiKey: 占位值 "cherry-managed-runtime-key", api, headers, models: [只有选中的那一个模型]}`。
- 注册到一个内存里的 `ModelRuntime`（`InMemoryCredentialStore`、`modelsPath: null`），真实密钥用 `setRuntimeApiKey` 传入。注释写明“只在内存中，从不使用 pi 全局的 auth.json / models.json”；`settings.json` 也用 `SettingsManager.inMemory` 代替。
- 协议映射（`src/shared/ai/piModelCompatibility.ts:55-91`）：Anthropic Messages → `anthropic-messages`，OpenAI Responses → `openai-responses`，OpenAI Chat → `openai-completions`，Gemini → `google-generative-ai`，Azure Responses → `azure-openai-responses`；Azure Chat、Bedrock、Vertex 等签名请求不支持。模型同时声明 Chat 和 Messages 时优先 Messages。
- 地址规则：OpenAI 类用 `formatApiHost`；Anthropic 去掉末尾的 `/v1`（“SDK 会自己拼 `/v1/messages`”）；Gemini 补 `v1beta`。
- 请求头：值里的 `$` 转义成 `$$`、开头的 `!` 转义成 `$!`，因为 pi 会把 `$ENV`、`!命令` 当模板解析。
- 模型字段：`reasoning`、`input`（可识图时加 `image`）、`contextWindow`、`maxTokens`（默认 8192）、`cost` 全 0（Cherry 自己计费）、`thinkingLevelMap`（只映射模型声明过的档位，否则 pi 会发 `medium`，严格的端点会拒绝）。
- **内部网关**：Cherry 云服务和 pi 不支持的协议走本机回环网关（`127.0.0.1`、随机端口、内存中的随机令牌 `cs-internal-<uuid>`）。网关提供 `/v1/messages`、`/v1/chat/completions`、`/v1/responses`、`/v1beta`、`/v1/mcps/:id/mcp`；模型名写成 `服务商id:模型id`，网关据此找回真实服务商，用 Cherry 自己的 SDK 和真实密钥转发。真实密钥不离开主进程。
- **变更生效**：把智能体配置、服务商和模型行、启用的密钥、MCP 集合、网关地址等算一个 SHA-256 签名；签名或思考档位变了就重建连接，空闲时立即重建，运行中等到下一轮。
- **MCP**：Cherry 的 MCP 服务器经 pi 的 MCP 扩展以内存传输接入；同一个桥也以 Streamable HTTP 暴露在网关的 `/v1/mcps/:id/mcp`。
- 打了 pi-ai 补丁：增加 `ultra` 档位；让 Gemini 适配器也能用自定义 fetch（走代理）。

### 4.2 DSH：写入 DSH 自己的配置，再启动 DSH 网页版

代码在 `src/main/services/deepSeekHarness/`。Cherry 不用 ACP，而是启动 `dsh web --host 127.0.0.1 --port 0 --no-open`，从标准输出读到地址后，在 Cherry 的小程序窗口里打开 DSH 自己的网页界面。

- 每次启动前改写用户真实的 `~/.dsh/settings.yaml`（`DSH_HOME` 指向 `~/.dsh`，与用户自己的 DSH 共用）：
  - `llm-pi-ai.providers.cherry-studio-codemate-<hash>`：`apiKeyEnv`、`displayName`、`api`（只支持 `anthropic-messages`、`openai-responses`、`openai-completions`）、`baseURL`、`models`（按 id 增改，保留其他条目；含 `contextWindow`、`maxTokens`、`input`、`reasoningEfforts`）。
  - `agent-default-model: {provider, model}`；另写 `agent-presets.default`。
  - `<hash>` 是 `sha256(服务商id + "\0" + 协议)` 的前 12 位；密钥变量名 `CHERRY_STUDIO_CODEMATE_<HASH>_API_KEY`。
- 密钥值写进 `~/.dsh/.credentials.yaml` 的 `refs.<变量名>`，`settings.yaml` 只通过 `apiKeyEnv` 引用变量名；不经环境变量、命令行或 `.env` 传递。启动前从继承的环境中删掉同名变量。
- 写入规则：文件锁（记录 pid，可回收死进程的锁）；在原 YAML 上就地修改，保留注释和无关键；原子写入，文件 0600、目录 0700；启动失败时按“回执”回滚，前提是期间没有别人改过。路由已存在但 `apiKeyEnv` 不是自己的，就拒绝覆盖。
- 不支持直连的服务商（登录型、只有 Gemini 协议的等）改走用户可见的统一网关：`http://<host>:<port>/v1`、`openai-completions`，模型名 `服务商id:模型id`，密钥用网关自己的 `cs-sk-<uuid>`。
- 界面只有两个下拉：默认 Agent 模式（继承 / 标准 / PTC 代码 / 极简）和默认权限（只读 / 工作区写入 / 完全访问，选完全访问时显示警告）。服务商和模型在通用的 Code CLI 页面里选。

### 4.3 其他 CLI 的通用做法

Claude Code、Codex、Gemini CLI、OpenCode 等同样是把服务商写进各自的原生配置文件（如 Claude Code 的 `settings.json` 中 `env.ANTHROPIC_BASE_URL`、`ANTHROPIC_API_KEY`），启动时只注入 PATH 等运行环境，不通过环境变量传密钥。

### 4.4 Claude 协议与服务商数据模型

- “Claude 协议”不是一种服务商类型，而是一种**端点类型** `anthropic-messages`（`packages/provider-registry/src/schemas/enums.ts:15-33`，另有 `openai-chat-completions`、`openai-responses` 等）。
- 服务商有 `endpointConfigs[端点类型] = { baseUrl, … }` 和 `defaultChatEndpoint`；模型有有序的 `endpointTypes[]`，第一个是默认（`src/shared/data/types/provider.ts:172-282`、`model.ts:370-450`）。同一个中转同时提供 OpenAI 和 Claude，就是一个服务商配两个端点，例如 aihubmix：Claude 用 `https://aihubmix.com`，OpenAI 用 `https://aihubmix.com/v1`。
- 用户新建自定义服务商时，默认提供的端点是 OpenAI Chat 和 Anthropic，其次是 Responses、Gemini（`ProviderEditorDrawer.tsx:63-66`）。
- 选端点：调用方偏好（模型声明了且服务商有地址）→ 模型第一个端点类型 → 按模型 id 的网关路由 → 服务商默认端点（`src/main/ai/provider/endpoint.ts`）。
- 地址规则（`src/shared/utils/api/format.ts`）：`formatApiHost` 去掉首尾空白和末尾 `/`，不含 `/v<数字>` 时补 `/v1`；末尾 `#` 表示原样使用（界面提示“在末尾添加 # 以禁用自动附加的API版本。”）。应用内对话用 `…/v1` 再拼 `/messages`；交给 Claude Code、pi 时用 `withoutTrailingApiVersion` 去掉 `/v1`，“SDK 会自己管理版本”。
- 鉴权：Anthropic 官方的模型列表请求用 `x-api-key` + `anthropic-version: 2023-06-01`（“用 Bearer 会 401”）；其他服务商（含 Claude 中转）同时发 `Authorization: Bearer` 和 `X-Api-Key`。Claude Code 子进程同时设置 `ANTHROPIC_API_KEY` 和 `ANTHROPIC_AUTH_TOKEN`。额外请求头合并到 `ANTHROPIC_CUSTOM_HEADERS`。
- 思考：Claude 4.5 及更早用 `thinking: {type: "enabled", budget_tokens}`（默认 13312，必须小于 `max_tokens`）；4.6 及以后用 `thinking: {type: "adaptive"}` + `effort`；Claude 4.5 推理模型加 `anthropic-beta: interleaved-thinking-2025-05-14`。
- 图像：`isVisionModel = 能力含 image-recognition 或输入模态含 image`（`src/shared/utils/model.ts`）；只有视觉模型才发图片，Claude 侧只支持 jpeg / png / gif / webp 的 base64 图片块，其他格式以本地路径写进文本。能力来自内置模型库、服务商覆盖、用户覆盖三层；未知模型默认没有任何能力，**没有按名字猜视觉能力**，用户在模型编辑里用“视觉”“推理”“工具”开关自行声明。
- 界面（zh-CN 文案）：“API 地址”“Anthropic API 地址”（提示“仅当服务商提供 Claude 兼容的基础地址时填写。”，并显示“Anthropic 预览：{{url}}”）、“更多端点”“添加端点”、模型的“端点类型”多选、“检测”。“检测”用所选模型发一次最小请求，15 秒超时、不重试。
- Cherry 另有 **Claude Code 运行时**（`src/main/ai/runtime/claudeCode/`，用 Claude Agent SDK）：只有当所有模型槽位都在主服务商上、且都解析到 `anthropic-messages` 时才直连，否则整个会话改走网关；通过 `ANTHROPIC_BASE_URL`、`ANTHROPIC_MODEL`、`ANTHROPIC_DEFAULT_*_MODEL`、`CLAUDE_CONFIG_DIR` 等环境变量接入。

### 4.5 API 密钥：显示、多密钥与轮换

显示（`src/renderer/pages/settings/ProviderSettings/ConnectionSettings/ApiKey.tsx`）：

- 已保存的密钥默认显示掩码：`maskApiKey`（`src/shared/utils/api/format.ts:43`）长度大于 8 时为“前 2 位 + `****` + 后 4 位”，否则显示 `••••••••`；等宽字体。
- 设置页只显示第一个已启用的密钥，多于一个时后面显示“+N”。行内按钮：显示 / 隐藏（眼睛图标）、复制（复制明文）、编辑、删除（只有一个密钥时）。切换服务商时恢复为掩码。
- 编辑时是密码框，同样有显示 / 隐藏切换。

多密钥（`ProviderApiKeyListDrawer.tsx`、`src/shared/data/types/provider.ts:61-72`）：

- 每个密钥一条：`{id（UUID）, key（去首尾空白，不能为空）, label（可选）, isEnabled}`；同一服务商内密钥值不能重复（“API 密钥已存在”），空值报“API 密钥不能为空”。
- 抽屉标题“API 密钥管理”，说明“管理当前服务商的多个 API Key”；每行显示标签（未填为“API Key”）、掩码、复制、编辑、删除和启用开关；底部“添加 API 密钥”（一行标签 + 一行密钥，“每次添加一个 API 密钥”）；页脚“N / M 已启用”。没有密钥时显示“API 密钥未配置”。

轮换（`src/main/data/services/ProviderService.ts:674-712`）：

- `resolveApiKey` 只在**已启用**的密钥里选：0 个返回空；1 个直接用；多个时按轮询，进程内缓存记住上一次用的密钥 id，下一次取它后面的一个，到末尾回到第一个。缓存不落盘，重启后从第一个开始。
- 显式指定的密钥（如“检测”某一个）不参与轮换。
- 轮换在**真正发请求时**推进：应用内对话每次请求选一次；pi、Claude Code 这类会话在**建立连接时**选一次，之后整个会话用同一个密钥。校验、签名计算等不推进轮换（代码注释多处强调“side-effect free”）。
- 会话签名只哈希**已启用密钥的集合**，不是本次选中的那一个，所以轮换本身不会触发重建；增删或启停密钥会。
- “检测所有密钥”：对每个已启用密钥分别发一次检测请求，结果逐条显示标签、掩码、通过 / 失败、耗时和错误信息，每条带启用开关，失败的可以当场停用（`ModelList/ApiKeyCheckResults.tsx`）。

### 4.6 对原生版的启示

- 两条路线：**投影**（把服务商翻译成引擎自己的配置格式）和**网关**（宿主在本机起一个多协议代理，引擎只连宿主）。Cherry 两条都用：能直连的投影，不能直连的走网关。
- 投影要按引擎各写一份；可直接借用 Cherry 的协议映射表、地址规则、请求头转义、档位映射。
- 网关能覆盖引擎不支持的协议，还能让真实密钥不进入引擎进程；代价是宿主要实现多协议转换。
- 管理型条目用稳定的哈希命名，并以 `apiKeyEnv` 作为“归属标记”，不覆盖用户自己的条目。
- 配置变化用签名判断，引擎空闲时重启。
- Cherry 的 DSH 和用户共用 `~/.dsh`；原生版按档案隔离配置目录，可以避免这一点。

## 5. 对标差距

| 原版能力 | 原生版能否直接对标 | 原因 |
|---|---|---|
| 设置外壳：左侧导航、分组、行布局 | 能 | gpui-component 已有对应控件 |
| 会话模型 + 力度 | 能，需新接口 | 原版写 ThinkRail 配置后由进程内 pi 解析；原生版要把选择翻译成三个引擎各自的默认模型配置，或在会话启动后用 RPC / ACP 设置 |
| 可见模型 | 能，需新接口 | 原版的模型表来自 pi 运行时；原生版要从引擎的 `get_available_models` / `configOptions` 读，或由宿主保存的提供方定义生成 |
| 评审模型、Agent 触发的评审、自动修复循环 | 不能 | 原生版没有评审功能 |
| GPT 上下文窗口 | 只对 pi 有意义 | 依赖 pi `models.json` 的 Responses 类模型覆盖 |
| 自定义服务器（中转） | 能，需扩展 | 原版只写 pi 的 `models.json`、只支持 OpenAI 兼容；原生版要为三个引擎各写一份，还要支持 `anthropic-messages` |
| 订阅登录（OAuth）、API 密钥登录 | 不能直接对标 | 原版调用进程内 pi 的 `runtime.login`；原生版的引擎是子进程，需要各引擎自己的登录流程 |
| JetBrains AI 卡片 | 不能 | 依赖 JetBrains central CLI |
| MCP 用户级服务器：增删改、启用、预设、JSON 导入 | 能，需新接口 | pi、DSH 可在启动时注入；OMP 只能写 `mcp.json` |
| MCP 项目级服务器、审批、工具授权 | 部分 | 原版依赖 `projects.json` 和 pi 的项目信任；OMP 还会自行读取项目里的外部 MCP 配置 |
| MCP 运行状态、工具数、日志、重新连接、OAuth | 不能直接对标 | 原版读取进程内 pi 会话的状态；原生版需要各引擎上报，目前三个协议里都没有对应事件 `[INFERENCE]` |

## 6. 待决问题

1. **API 密钥存哪里。** 原生版现有设计不保存密钥（多 Agent 设计包 01 Q05、`validate.rs`）；原版存在宿主本机的 `models.json`；Cherry 存在自己的数据库里，给 DSH 时写进 `.credentials.yaml`。可选：只填环境变量名；存入 Windows 凭据管理器，启动引擎时注入；明文写入配置文件。
2. **投影还是网关。** 投影：宿主把服务商翻译成三种引擎配置（pi 用 `-e` 扩展或 `models.json`，OMP 用 `models.yml`，DSH 用 `--patch`）。网关：宿主起本机多协议代理，引擎只连宿主，真实密钥不进引擎进程，也能支持引擎本身不支持的协议，但宿主要实现协议转换。Cherry 两者结合（第 4.6 节）。
3. **提供方与 MCP 的作用范围。** 全局一份、所有档案共用，还是每个档案各配一份。原生版的配置目录按档案划分。
4. **宿主是否写引擎的配置目录。** 不写文件的方式（pi `-e`、DSH `--patch`）对 OMP 的 MCP 不够用；写文件则会改动用户的引擎配置，引擎 CLI 也会看到。若写，可沿用 Cherry 的规则：哈希命名、归属标记、就地修改、原子写入、失败回滚。
5. **支持哪些协议。** 原版只有 `openai-completions`；三个引擎和 Cherry 的 DSH 直连都只支持 `anthropic-messages`、`openai-completions`、`openai-responses`。
6. **范围。** 订阅登录、JetBrains AI、评审、GPT 上下文窗口、MCP 项目级审批与运行状态这几项在原生版没有对应基础。

## 7. 未核实

- pi 在测试环境中的 easyclaude 中转是怎么配置的（隔离目录中的 `auth.json`、`models-store.json` 都是空的）。
- OMP 在 RPC 模式下 `-e` 扩展的 `registerProvider` 是否生效。
- OMP 在 RPC 模式下是否有其他路径会触发 `models.yml` 刷新。
- DSH 的 `llm.listProviders()` 只返回已配置的路由，还是整个内置目录。
- 三个协议里是否有 MCP 服务器状态事件。
- Cherry：DSH 在运行时如何读取 `apiKeyEnv` 与 `.credentials.yaml`、是否热重载 `settings.yaml`；Cherry 安装的 DSH 具体版本（预设只写了允许预发布）。
- 以上都来自源码和文档阅读，除第 3.2 节 DSH 一行外均未实际运行。
