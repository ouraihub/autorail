# 构建

工具链由 `rust-toolchain.toml` 固定为 Rust 1.99.1，依赖已精确锁定在 `Cargo.lock` 中。

Windows 上统一用便携 MSVC（MSVC 14.44 + Windows SDK 26100，x64）构建，即使已经装了 Visual Studio Build Tools 也一样。脚本在 `scripts/msvc/`，取自 [`@ouraihub/desktop-kit`](https://github.com/ouraihub/desktop-kit)（MIT，许可见 `scripts/msvc/LICENSE`），与 AutoNote 共用同一套工具链。

首次使用，在仓库根运行一次（下载约 455 MB 的微软构建组件，需要 `uv` 和 `curl`）：

```powershell
scripts\msvc\setup-msvc.bat
```

默认安装到 `D:\Applications\msvc-portable\msvc`，整台机器只需一份。装到别处时设置 `PORTABLE_MSVC_ROOT` 指向含 `setup_x64.bat` 的目录。

之后所有 cargo 命令都经 `cargo-msvc.bat` 执行，参数原样传给 cargo：

```powershell
scripts\msvc\cargo-msvc.bat build                      # 开发构建
scripts\msvc\cargo-msvc.bat fmt --all                  # 格式化
scripts\msvc\cargo-msvc.bat test --workspace           # 单元测试与集成测试
scripts\msvc\cargo-msvc.bat clippy --workspace --all-targets -- -D warnings
scripts\msvc\cargo-msvc.bat run -p thinkrail-native -- --data-dir D:\tmp\tr-data
```

`rust-toolchain.toml` 固定了 `rustfmt` 和 `clippy` 组件，rustup 会自动安装。GitHub Actions 的 `ci` 检查在每次 push 和 PR 上依次运行 `fmt --all -- --check`、clippy 和 `test --workspace`，用的是和本机相同的命令；推送前先在本机跑一遍。

## 打包 EXE

从仓库根目录运行：

```powershell
scripts\msvc\package-exe.bat
```

脚本会使用便携 MSVC 构建 `--workspace --release`，然后生成：

```text
dist\thinkrail-native\thinkrail-native.exe
dist\thinkrail-native-x64.zip
```

也可以传版本号，让 ZIP 带版本名：

```powershell
scripts\msvc\package-exe.bat 0.1.0
```

发布包不内置用户数据；首次运行默认使用 `%USERPROFILE%\.autorail`，也可通过 `--data-dir <目录>` 隔离数据。

需要真实进程的测试默认标为 ignored，按需运行：

```powershell
scripts\msvc\cargo-msvc.bat test -p thinkrail-terminal windows_pty_lifecycle_smoke -- --ignored --nocapture
# 隔离配置、占位模型，无凭据、不发模型请求。按实际安装位置设置：
$env:THINKRAIL_SMOKE_PI = 'C:/path/node.exe|C:/path/pi-coding-agent/dist/bundle/cli.js'
$env:THINKRAIL_SMOKE_OMP = 'C:/path/omp.exe'
$env:THINKRAIL_SMOKE_DSH = 'C:/path/node.exe|C:/path/dsh/lib/bin.js'
scripts\msvc\cargo-msvc.bat test -p thinkrail-agents smoke:: -- --ignored --nocapture --test-threads=1
```

它先注入便携 MSVC 的 `PATH`、`LIB`、`INCLUDE`，并检查 `cl.exe`、`link.exe` 确实来自便携目录，再调用 cargo。直接运行 `cargo` 会用到 Git for Windows `usr\bin` 里的 `link.exe`（GNU coreutils 的 link 命令，不是链接器），报 `extra operand … Try 'link --help'`。Windows SDK 的 `rc.exe` 也来自这套工具链，`turso` 等依赖的构建脚本需要它。

## 换行符

`.gitattributes` 规定：普通文本文件在仓库和工作区都用 LF；`.bat` / `.cmd` 用 CRLF，并且只能是纯 ASCII，否则 cmd.exe 会解析错。`.editorconfig` 让编辑器按同样的规则保存。CI 的 `line-endings` 检查会拒绝仓库里存成 CRLF 的文本文件和含非 ASCII 字符的批处理文件。

这些规则不依赖本机的 `core.autocrlf`。在规则加入之前克隆的仓库，工作区里可能还留着 CRLF 文件；在工作区干净时运行一次：

```powershell
git read-tree HEAD; git checkout-index -f -a
```

## 已知问题

- **debug 构建移动 target 目录后窗口打不开**（`Creating DirectX render pipelines: 0x80070003`）：GPUI 的 debug 构建会从编译时记录的绝对路径读取 shader。重新编译这两个 crate 即可：`cargo clean -p gpui-fast-windows -p gpui-pre-windows`。release 构建会把 shader 编译进二进制，不受影响。

## 运行时要求

应用安装包不带 Agent 引擎。打开“设置 → 编码搭档”：已在 `PATH` 上的 `pi` / `omp` / `dsh` 会被检测到并标“系统”；没装的点“安装”，应用下载固定版本的 Node v24.22.2 或 Bun 1.4.4（校验 SHA-256），再把 pi 1.4.0、OMP 18.14.2、DSH 0.2.1-rc.5 装到 `<data>/coding`，标“内置”。“设为默认”决定新聊天用哪个引擎；新建项目或新建聊天时没装引擎、没设默认，界面会给出引导按钮直接打开这一页。引擎使用自己的默认配置目录（pi `~/.pi/agent`、OMP `~/.omp/agent`、DSH `~/.dsh`）。中转地址、模型和 API 密钥在“模型服务”中配置；每个引擎用哪个服务商在“编码搭档”里选，也可以选“{引擎} 官方”继续用引擎原有的模型配置。

模型、提供方、密钥、MCP、默认 Agent 和聊天绑定存在 `<exe>/data/thinkrail.db`；显式 `--data-dir <目录>` 时数据库在该目录。项目与日志默认位于 `%USERPROFILE%/.autorail`，不读取或自动迁移 `.thinkrail`、`.thinkrail-native` 的数据；旧开发版的 `agent-profiles.json` 不再读取，旧版本号的 `thinkrail.db` 需要删除后重建。API 密钥按用户选择明文存储，复制/显示才取出；请保护数据库文件。所有项目共用 OMP 的默认配置目录，MCP 集合以最后启动的项目为准。

模型选择器使用 LumaKit 的 `crates/lumakit-model-picker`，以 Git 依赖固定到 GitHub 上 LumaKit 的提交（见根 `Cargo.toml`，更新步骤见 `AGENTS.md`），不依赖 LumaKit 的 AI／数据库／设计系统。入口位于聊天输入框下方，浮层向上展开；顶部不再保留模型和力度的重复下拉。每个模型的说明为“{路由} · {上下文} 上下文”，上下文格式与设置页共用 `settings_models::format_context`（`1M`、`272K`）。图标为原 ThinkRail 使用的 Remix 4.9.0 SVG，许可保存在共享 crate 的 `LICENSE-REMIX`。

2026-11-05 验证：workspace 280 项通过，原有 16 项下载／外部引擎测试保持 ignored；fmt 与严格 clippy 通过。隔离 `--data-dir` 和 `USERPROFILE`、真实 pi 会话中操作了分组、附加 high 的搜索、力度点击／拖动／键盘、默认值存储、重置 off、刷新和 Esc。未发送模型请求，不冒充上游回答验证。

2026-11-07 模型页第一期（agent-settings 08 R1、R2）验证：workspace 293 项通过（16 项 ignored 不变），fmt 与严格 clippy 通过。隔离 `--data-dir` 和 `USERPROFILE`、从本机数据库复制并去掉密钥的副本中，Windows 实窗操作了：会话模型与力度单行选择并重启后保持；可见模型展开；GPT 上下文窗口无合格模型时显示“没有符合条件的模型”，有 Responses 模型时全局选 1M、单个模型自定义 400000（非法值标红不保存）、全局随之显示“按模型分别设置”，均在重启后保持。未发送模型请求。

上下文状态依赖同一 LumaKit 提交中的 `lumakit-usage-status`，位于发送按钮左侧。Pi/OMP 在后台查询原生 `get_state`／`get_session_stats`，不从截断的 UI 文本推算；DSH 无报告时保持未知。Native 没有宿主模型价格字段，因此只显示 token，不套用目录参考价；若未来引擎提供用户配置价格快照，才显示金额。Windows 实窗已用真实 pi、OMP 连接隔离回环 HTTP 验证输入 1000（含缓存读取 200）、输出 50、累计 1050；未连接收费服务。共享系统图标保留 macOS/Linux 映射但未做该平台运行验收。

2026-11-09 日志链路续作：workspace 332 项通过、0失败，16项既有外部依赖 smoke 保持 ignored；fmt、all-targets check 与严格 clippy 通过。使用隔离 data-dir / USERPROFILE、真实已安装 Pi 与本机回环 OpenAI 服务观察成功、HTTP400失败及生成中停止；turn 分别写 EXIT/ERROR/CANCEL，失败/停止不会被后续 settle 改写为 SUCCESS。真实子进程请求20ms超时回归验证 TIMEOUT/Limit。

后台日志支持 EV/Layer 交集筛选；后台任务失败通知和引擎失败均提供复制真实诊断 ID。实窗复制后粘贴筛选，选择「失败 + service」得到对应 turn 的唯一失败行。外网供应商未连接，OMP/DSH本次未做真实发送。完整记录见相邻 LumaKit 的 `docs/Rust-LumaKit-日志-请求-响应-异常封装任务列表.md`。

## 2026-11-10 共享密钥管理联调

Native 密钥管理使用共享 `key_entry` 的标签／密钥／提交行和内容自适应的 `compact_form_body`，宽度取 `size::SETTINGS_DIALOG`；已保存行直接使用共享 `KeyRow::label`，不再嵌套另一套标签容器。标签可省略，编辑时留空密钥只修改标签；异步写入成功后才清空，失败保留输入与编辑目标。

本轮通过便携 MSVC wrapper、外部临时 `--config C:\Users\Administrator\AppData\Local\Temp\shared-provider-ui-cda1gesx\native-patch.toml` 联调未发布的 LumaKit；仓库不加入本地 path patch。定向验证：

```powershell
scripts\msvc\cargo-msvc.bat test -p thinkrail-native panels::settings::providers --config <临时配置> -- --test-threads=1
scripts\msvc\cargo-msvc.bat test -p thinkrail-native --test providers --config <临时配置>
scripts\msvc\cargo-msvc.bat clippy -p thinkrail-native --all-targets --config <临时配置> -- -D warnings
scripts\msvc\cargo-msvc.bat build -p thinkrail-native --config <临时配置>
```

结果：提供方 UI 单元回归 5 项通过（包含本轮新增的成功后清空、无标签添加、仅改标签不改密钥、失败保留输入及编辑目标），公开服务集成测试 1 项通过，严格 clippy 与 debug build 通过。未以本轮定向验证替代整个 workspace 验证。

实窗使用新的 `C:\Users\Administrator\AppData\Local\Temp\native-keys-finish-l9kw_acv\data` 和同级 `profile`，以 `--data-dir`／`USERPROFILE` 隔离；仅创建假密钥与 `http://127.0.0.1:1/v1` 自定义提供方，不访问外部服务、不读取运行中数据库。通过 UI 添加带标签密钥、仅改标签、替换密钥，关闭并重启后重新打开提供方确认标签和替换值保留。主行悬停显示复制／编辑／眼睛，无删除；管理行显示标签／掩码／复制／编辑／删除／启用，无眼睛。新建时只填标签提交显示“API 密钥不能为空”且保留标签；真实持久化失败由隔离真实服务回归覆盖，未在实窗注入磁盘故障。

截图根目录：`C:\Users\Administrator\AppData\Local\Temp\native-keys-finish-l9kw_acv\screenshots`。保存／编辑／重启证据为 `manager-saved-wide.png`、`manager-label-only-saved.png`、`manager-replacement-saved.png`、`manager-restarted-persistence.png`、`main-restarted-secret-persistence.png`；主行按钮为 `main-key-actions-final.png`；校验失败为 `key-manager-validation-failure-320.png`。各宽度原始截图为 `settings-provider-{320,375,414,768}.png` 与 `key-manager-{320,375,414,768}.png`，均为真实窗口尺寸，不是截图缩放。

首轮窄屏观察：320／375／414 的设置导航换行、内容可滚动；768 导航保持侧栏、提供方列表与详情上下堆叠。管理输入与提交按钮在不足宽度时换行，关闭和各操作可达。但首轮 320 的管理行标签挤掉掩码，主行长标签也挤掉掩码，故这些截图不是“窄屏视觉全部通过”的结论。

随后共享 `KeyRow` 保留掩码最小宽度并允许行内动作换行，Native 管理行也允许启用开关换行；再次运行严格 clippy／debug build 通过。修正后重新实窗捕获 `settings-provider-final-{320,375,414,768}.png`、`key-manager-final-{320,375,414,768}.png` 和 `key-manager-final-wide.png`：320 的标签、掩码可见，复制／编辑／删除／启用和关闭均可达；主行的眼睛在短宽度时换到行内第二行，375／414 保持可用，768／宽屏管理输入为一行。截图显示长标签／掩码按可用宽度截断，不承诺完整明文可读；查看／复制仍可用。中间的 `*-corrected-{375,414,768}.png` 被桌面截屏选择浮层遮挡，不作为验收证据，最终使用上述 `*-final-*`。本轮自行启动的调试进程均正常关闭，没有结束其他会话。


之后按原始参照将真实 enabled `Switch` 移到共享 `KeyRow::trailing`，使开关与标签／掩码／动作处于同一边框内，删除 Native 外层行包装；原有保存语义保持不变。该调整通过 wrapper `fmt`、定向 `check -p thinkrail-native` 和 debug build。最终实窗证据为同目录 `key-manager-bordered-switch-320.png` 与 `key-manager-bordered-switch-wide.png`：320 内标签／掩码一行，复制／编辑／删除／开关在边框内第二行，宽屏同一边框内单行。实际点击开关关闭／开启，计数分别显示 `0 / 1 已启用`、`1 / 1 已启用`，截图为 `key-manager-bordered-switch-disabled-320.png`、`key-manager-bordered-switch-enabled-320.png`。自启进程 PID 24312 正常退出；最终远程 Git pin 的构建仍由集成验收记录补充。

最终依赖已固定到推送后的 LumaKit `f263608cdc5efcb107ab7d3d060a9caba85d0afa`，所有 LumaKit rev 一致；不需要本地 patch。无临时配置的 all-targets check、fmt、宿主严格 clippy、5项提供方 UI回归通过。临时 patch 阶段 workspace严格clippy和全部测试通过，16项既有下载/引擎/ConPTY测试仍按原约定ignored。锁文件只保留新增共享组件及LumaKit来源变化，不升级无关依赖。

## 2026-11-10 日志异常整改与同步闪退

后续人工/AI 开发先读[日志与异常规范](日志与异常规范.md)；AGENTS 与 SPEC 已接入，04/06 日志设计、Trace API 和模型探测错误合同已同步，旧 op/child 模板不再适用。

| 已发现问题 | 整改 |
| --- | --- |
| 同步成功打开 picker 时读取正在 update 的 ProvidersPage | 传捕获的 provider ID，不重入 page.read；保留 stale guard、重复请求禁用、失败可重试；模型添加使用可访问原生按钮 |
| panic 只留下 UNFINISHED，退出前异步队列可能未写完 | 独立同步 ERROR 层与原滚动文件共用 writer，记录位置/线程/栈/安全哈希；不原样输出 payload，不捕获后假装成功 |
| UI report、后台 root、tool wrapper 重复记录/平行 trace | root 负责最终 ERROR；notice 只显示已有诊断 ID；跨 session/terminal/plan channel 传真实 span，tool wrapper 继承调用者 |
| startup process::exit 跳过日志 Drop，raw anyhow 泄漏 | 稳定 code 与诊断 ID；服务失败释放 logger 后退出；字体可恢复降级记 WARN |
| 协议 send/read、线程开始等 INFO 噪音 | 降 DEBUG，仅安全结构/大小；生命周期仍 INFO，异常不降级伪成功 |
| DB SQL、Git stderr、上游 body、key label、技能绝对路径泄漏 | 稳定分类/status/内部 ID 替代原文；DEBUG 亦禁密钥/正文/绝对路径 |
| 清理/事务恢复/channel/thread 失败被丢弃或误报成功 | 明确异常责任、清理失败摘要、事务提交后再报告完成；worker panic 保持传播；不完整 spec 索引拒绝读写 |
| 日志 filter 解析失败静默 fallback | 无效配置拒绝初始化，不改变用户选择 |

本地验证命令（均经便携 MSVC wrapper）：`fmt --all -- --check`、`clippy --workspace --all-targets --locked -- -D warnings`、`test --workspace --locked`、`build -p thinkrail-native --locked`。352 项通过，16 项既有外部引擎/下载/ConPTY ignored 未改。新增回归覆盖同步真实 HTTP + GPUI picker 添加持久化/失败重试、panic abort 前落盘/off 过滤/脱敏、DB commit 失败、Git index 恢复失败、MCP worker 写失败/panic、部分 spec 索引拒绝、安装清理和 stderr 读取失败。panic release 定向 3 项通过；release 缺少符号时允许地址栈，不保证函数名。

实际 debug 程序使用隔离 `C:/Users/Administrator/AppData/Local/Temp/native-log-audit-4efuo0fq/` 与本地回环，观察同步成功、HTTP 429 失败、重试成功且窗口继续存活，截图 `sync-success.png` / `sync-failure.png` / `sync-retry.png`。失败 TraceId `fe38f608bc726771845c3cb428a96149` 只有一个最终 ERROR，成功均 ENTRY/EXIT；日志无测试上游正文。正式数据与真实供应商未访问。

CI run `42750421487` / job `129617640445` 的真实失败是 `real_process_readers_and_reply_callbacks_keep_sessions_isolated_at_info`：两个 PowerShell 冷启动被算入 20 秒 RPC 时限，收到 Timeout。测试现先等待子进程输出 ready 帧，再开始原 20 秒 RPC；不延长产品超时、不忽略回归。定向与本地全 workspace 均通过，不能据此宣称 GitHub 新运行已通过。

边界：Gateway 只能真实关联 token 注册的 session/setup span，没有可信 per-turn header，不能宣称当前 turn 全链路已贯通；Agent 使用会话关联 TraceId 与 child turn span。Rust hook 不覆盖系统强杀、访问违规、栈溢出/断电；文件系统无法写入时不能保证诊断落盘。未重新发送真实 Pi/OMP/DSH 模型请求，历史 smoke 不是本轮全部复测。

模块规模评审：日志入口145行/panic67行；gateway按监听与转发拆为292/169行，Git按领域查询/进程生命周期拆为338/155行。main197/state299/services357行，资源与错误边界未复制。既有大模块本轮保留窄改动：provider存储457行（同一事务/CRUD所有者）、spec工具436行（同一公开工具合同）、Projection403行（同一事件状态机）、日志页402行（同一读取/筛选投影）、picker409行（同一选择草稿/保存所有者）；其中少量增长作为本轮错误边界修复例外，不为分行扩大公开接口或拆散状态所有权。其余既有超过阈值文件未新增行为，不将此记录当作已完成全仓规模重构。

整改代码 `0de6d2c` 已推送 main；`package-exe.bat` 已重建原发布目录与 ZIP，包内 EXE/发布 EXE SHA-256 一致：`ea77957c9530eeabb14d1a39b3c187068dc96e9e171115835cd58a43691f6fbc`。release 实窗同步打开 picker，选中模型后重启并再次选择原服务商，`audit-model-safe` 仍在模型列表；截图 `release-sync-success.png`、`release-sync-model-added.png`、`release-restart-selected-provider.png`（同上隔离目录）。保存证据为应用自身重启读取及真实服务持久化回归。

GitHub 新运行 [42756799363](https://github.com/ouraihub/autorail/actions/runs/42756799363) 在 `0de6d2c` 的 Format、Clippy、Test 全部成功，rust job 129632937278 用时4分55秒；原失败运行保留历史，不重跑旧提交。现有 action 的 Node 20 弃用提示为非失败注解，本轮未无关升级 action。
