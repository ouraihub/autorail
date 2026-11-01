# 06 日志与调用链

状态：**已实现**（2026-11-02）。目标：用户在界面上的每次操作，都能在日志里看到完整的调用链；日志行可以直接复制给大模型用来定位问题。参考了 AutoNote / Aether 的 `aether-observability` 和 AutoNote 的"显示后台日志"。日志规则本身见 [04 §3](04-detail.md#3-日志规范)。

## 1. 结构

| 位置 | 职责 |
|---|---|
| `crates/observability`（`thinkrail-observability`） | 安装订阅器、按日滚动写文件（保留 14 个）、北京时间时间戳、可随时切换级别（`LogHandle::set_level`）、`op` span、`tail` 读取与行解析 |
| `core::app_prefs` | `AppPrefService`、`AppPrefs { logs_visible, log_level }`、`LogLevel` |
| `settings-store` | `app_prefs` 表（库版本 5），实现 `AppPrefService` |
| `app::state::run` | 每次操作建一个 `op` span |
| `app::panels::settings::logs` | "后台日志"设置页 |

与 AutoNote 的分工相同：日志相关能力都在独立的日志 crate 里，`app` 只调用它。宿主自己的偏好与业务设置分开存放；这里放在同一个数据库的单独一张表 `app_prefs`，而不是放进 `model_prefs`。

## 2. 调用链：`op` span

- **只改一处**：面板里需要读写数据或调用引擎的操作，都经过 `state::run(cx, job, done)`。`run` 标了 `#[track_caller]`，为每次调用建一个 span：`op{id=<递增编号> at=<发起位置，形如 panels/chat.rs:312>}`。
- **覆盖范围**：后台任务 `job` 和回到界面后的 `done` 都在这个 span 里执行。这次操作里服务层、引擎启动、路由写的每一行日志，行首都带 `op{…}:`。
- **结束行**：结束时写一行 `op done ms=<耗时>`。成功时这行是 `debug` 级别，失败时是 `info` 级别并带 `ok=false`；失败的错误码由 `AppState::report` 另记一行 `warn`，也在同一个 span 里。
- **后台线程**：引擎的读线程、宿主工具调用不经过 `run`，靠 `chat=<id>` 字段关联：
  - `agent turn started` / `agent turn ended ms=…`，引擎在一轮对话中途退出时记 `agent turn cut off`（`agents::projection`，三个引擎共用）；
  - `host tool called tool=… ms=… is_error=…`，失败时记 `host tool failed code=…`（`app::services::NotifyingTools`，所有引擎的工具调用都经过这里，每次调用只记一行）；
  - 已有的 `agent session started`、`agent process exited code=…`。
- **按调用链查看**：在日志页的筛选框输入 `op{id=42`，就只剩这次操作的所有行；输入 `chat=c1`，就是这个聊天的引擎侧日志。

```text
2026-11-02T01:25:29.6+08:00  INFO op{id=10 at=panels/settings/logs.rs:116}: thinkrail_settings_store::app_prefs: log page visibility set visible=true
2026-11-02T01:25:31.6+08:00  INFO op{id=11 at=panels/settings/logs.rs:124}: thinkrail_settings_store::app_prefs: log level stored level="debug"
2026-11-02T01:25:31.6+08:00  INFO op{id=11 at=panels/settings/logs.rs:124}: thinkrail_observability: log level changed level="debug"
2026-11-02T01:25:31.6+08:00 DEBUG op{id=11 at=panels/settings/logs.rs:124}: thinkrail_observability::op: op done ms=13
```

已知限制：

- 筛选是子串匹配，`op{id=1` 也会匹配到 `op{id=12`。要精确匹配时，输入时带上后面的空格：`op{id=1 `。
- 在 `run` 之外直接 `spawn` 的少数地方，例如打开文件夹选择框，不带 op。

## 3. 文件与时间

- **文件**：`<data>/logs/thinkrail-native.YYYY-MM-DD.log`，每天一个文件，只保留最近 14 个。文件名里的日期按 UTC 计算，所以新文件在北京时间早上 8 点生成。
- **时间戳**：写入文件时就是固定的北京时间（东八区），格式为 RFC 3339，例如 `2026-11-02T01:25:31.6517747+08:00`，不依赖系统时区设置。用到的是 `time` crate 和 `tracing-subscriber` 的 `time` 功能。
- **级别**：
  - "信息"对应 `info`。
  - "调试"对应 `info,thinkrail=debug`：只放开本应用的 crate；数据库、渲染器等依赖库仍按 `info`，否则它们内部的调试输出会把日志页刷满。实测打开后依赖库的调试行从 877 行降到 0。
  - 设置了环境变量 `THINKRAIL_LOG` 时，以环境变量为准，页面上的级别设置不生效，并显示提示。
- **启动顺序**：日志先以 `info` 启动，因为数据库还没打开。`services::build` 打开数据库后读出保存的级别再切换过去，所以最早几行启动日志总是 `info` 级别。

## 4. 偏好：`app_prefs`

```sql
CREATE TABLE app_prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL);
-- logs.visible = true | false    显示后台日志（默认 false）
-- logs.level   = "info" | "debug"（默认 "info"）
```

数据库版本因此从 4 升到 5。按 agent-settings 07 D6 的约定不写升级步骤：开发期的旧 `thinkrail.db` 需要删掉后重建。

## 5. 后台日志页

设置导航里新增"后台日志"一项（`ScrollText` 图标），放在 MCP 服务器下面。

- **显示后台日志**：`Switch`。只有打开时才显示日志面板、才去读日志文件；关闭开关不影响后台写日志。
- **日志级别**：`Segment`（信息 / 调试），改完立即生效，并存进 `app_prefs`。
- **日志面板**：
  - 标题显示"最近 N 条 · 每 2 秒刷新"；
  - "只看问题"开关，只显示警告和错误；
  - "刷新"按钮；
  - "复制"按钮：把当前显示的原始行按时间从旧到新复制到剪贴板；
  - "打开日志目录"按钮（`cx.reveal_path`）；
  - 筛选框：按子串匹配原始行，可以输入 op、chat 或模块名。
- **每行显示**：时间（北京时间的时分秒）、级别、op 前缀（主色）、模块名（只显示 `::` 最后一段）、内容。错误标红，警告标黄。
- **读取方式**：`observability::tail` 只读最新日志文件的最后 256 KiB，最多 300 条：
  - 开头被截断的半行丢掉；
  - 文件末尾还没写完的最后一行丢掉；
  - 解析不了的行按 `INFO` 级别、整行作为内容显示。
- **只在打开时刷新**：只有开关打开、并且当前停在这一页时才刷新；离开这一页或关闭设置，刷新任务就被丢弃。日志页自己的刷新不经过 `run`，否则它每 2 秒会往日志里写一行 `op done`。

## 6. 验证（2026-11-02，隔离 `--data-dir`、真实窗口）

- **单元测试**：
  - `observability/tests/tail.rs`：op 前缀拆分，无 op 的行，无法解析的行，多个日期文件时取最新一个，截断与未写完的行，日志目录不存在。
  - `observability/tests/init.rs`：真实订阅器写出北京时间和 op 前缀，"调试"只放开本应用的 crate。
  - `settings-store/tests/agents.rs`：`app_prefs` 的默认值，重开数据库后保留。
- **窗口操作**：
  - 打开开关并选"调试"后，面板显示 op 行；在筛选框输入 `op{id=11` 后，只剩该操作的三行（写入、切换级别、`op done ms=13`）。
  - 点"复制"后，剪贴板里是这三行原始日志，按时间从旧到新。
  - 重启后开关和"调试"都保留，启动日志里有 `log level changed level="debug"`。
- **只在打开时读取**：用一个临时计数器统计读取次数，验证后已删除。
  - 停在其他设置页：0 次；
  - 停在日志页约 7 秒：4 次；
  - 离开日志页后 6 秒：不增加；
  - 关闭设置后 6 秒：不增加。
