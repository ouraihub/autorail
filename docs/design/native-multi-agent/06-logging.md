# 06 日志与调用链

当前开发合同见[日志与异常规范](../../日志与异常规范.md)，架构动机见[Native Trace 设计](../10-observability-trace.md)。本文只维护产品存储、显示和初始化行为，旧 `op{id}` / `op done` / `child()` 示例已删除，不再作为新增功能模板。

## 1. 职责

| 位置 | 职责 |
| --- | --- |
| `crates/observability` | subscriber、GrepFormat/SpanFields、日滚动文件、运行时级别、panic 同步诊断、tail |
| `core::app_prefs` / `settings-store` | 保存 logs_visible / log_level，不在服务库安装 subscriber |
| `app::state::run` | UI 后台操作 root，跨工作任务与完成回调传播 span |
| Agent/HostTool/Review 生命周期所有者 | 各自操作边界和线程/channel 传播，不能靠 chat ID 替代 TraceId |
| `app::panels::settings::logs` | 日志开关、搜索、EV/Layer 筛选、复制诊断 ID |

## 2. 调用链

`TraceContext::root("ui.background", "ui")` 返回 trace/guard，创建不会失败。span 的 SourceFile/SourceLine 对应发起消费者。job 与 done 使用同一 span；子服务日志继承同一 TraceId，显式子 operation 具有自己的 SpanId/ParentSpanId。

正常完成与失败由 guard 写唯一终态：ENTRY/EXIT/ERROR/CANCEL/TIMEOUT/UNFINISHED，RT 为毫秒。普通 UI 失败在 root 记一次 ERROR，UI 提示不重复报错。panic 是独立程序错误，hook 在展开前同步记录 ERROR，guard 的 UNFINISHED 是异常未完成事实。

按 `TraceId:<完整ID>` 搜索同一链路；ME 定位操作，Layer 定位层。日志页只做 parse_line 的字段投影，不另造解析器或 ID。

## 3. 文件、时间、级别

- `<data>/logs/thinkrail-native.YYYY-MM-DD.log`，UTC 日期轮换，保留 14 个；行时间为 RFC3339 `+08:00`。
- 默认 INFO；调试模式仅打开应用 crate 和 `lumakit_observability` 的 DEBUG，不放开数据库/渲染器全部细节。
- `THINKRAIL_LOG`（EnvFilter 语法）优先于设置。panic 专用 ERROR 不受 `off` 过滤。
- 日常日志使用非阻塞 writer，LogHandle 持有到正常退出。panic 同步写同一滚动文件；不能依赖队列 Drop，因为进程可能直接 abort。
- 初始化在服务和窗口前完成。debug 构建同时输出普通日志到 stderr；panic 不向默认 stderr hook 泄露任意载荷。
- 日志失败不阻断正常业务，不重试业务副作用。Rust hook 不保证处理访问违规/断电/系统强杀，应结合 Windows Application/WER 与匹配符号。

## 4. 偏好与日志页

`app_prefs` 保存 `logs.visible`（默认 false）、`logs.level`（info/debug）。关闭显示不停止日志写入。

日志页提供最近记录、刷新、复制当前筛选结果、打开目录、文本搜索、只看问题、EV/Layer 交集筛选；短 TraceId 可筛选、完整 ID 可复制。只在日志页打开时读取，刷新本身不走 state::run，避免日志自反馈。

tail 读取最新日志最后 256 KiB、最多 2000 完整行；丢弃开头截断半行和末尾未完成行，不可解析行仍显示，不虚构 trace。

## 5. 验证

- `observability/tests/init.rs`：真实文件输出、并行 trace 隔离、下层级别、INFO 过滤与正常 flush。
- `observability/tests/tail.rs`：字段、轮换选择、截断边界、非操作日志。
- `observability/tests/panic.rs`：隔离子进程中，off 级别下 panic ERROR、trace 关联、敏感载荷不落盘、abort 不依赖 guard drop。
- 功能新增还须按统一规范验证真实失败与终态，不能把“日志行存在”或旧实测记录当作新链路已验收。
