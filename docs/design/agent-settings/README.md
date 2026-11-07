# Agent 设置设计包：模型、提供方、MCP 服务器

状态：实施中，后端已完成并通过三引擎本机直连/网关回合验证；设置界面和会话接入待合并验证（2026-10-31）。属于 [多 Agent 设计包](../native-multi-agent/README.md) 的后续特性，沿用其约定（U01–U09、错误与日志规范、事实标记）。

目标：在设置的 AGENT 分组下实现“模型”“提供方”“MCP 服务器”三页，界面对标原版 ThinkRail；用户配置一次中转（OpenAI 或 Claude 协议，多密钥轮换），pi、OMP、DSH 三个引擎都能使用。

## 阅读顺序

1. [01 调研](01-research.md)：原版设置、原生版现状、三个引擎的配置能力、Cherry Studio 的做法。
2. [02 方案](02-plan.md)：用户决定 S01–S10、数据库选型、M0 实测结论、范围。
3. [03 概要设计](03-architecture.md)：crate 分层、状态归属、核心流程、安全。
4. [04 详细设计](04-detail.md)：core 契约、存储表结构、路由算法、网关、三个引擎的投影、界面。
5. [05 任务列表](05-tasks.md)：T1–T15、依赖、验收矩阵、交接模板。
6. [06 提供方交接设计](06-provider-handoff.md)：Provider UI 的代码分析结论、状态边界、后续任务与验收要求。
7. [07 默认 Agent 与新建项目引导](07-default-agent.md)：去掉 Agent 档案，默认 Agent 和聊天绑定存进数据库，新建项目时检查引擎并引导安装、设默认（设计，取代本包与多 Agent 设计包中的档案内容）。
8. [08 模型页对标原版](08-models-page.md)：默认模型行布局、评审组（评审模型、Agent 触发的评审、自动修复循环）及其后端、GPT 上下文窗口；原版机制分析与原生版设计（讨论中）。

| 位置 | 内容 |
|---|---|
| `crates/core` | 提供方、MCP、偏好、路由类型与 5 个 trait；`StartSession` 增加 `routing`、`support_dir` |
| `crates/settings-store`（新） | Turso 0.8.3 数据库：提供方、端点、模型、多密钥、偏好、MCP |
| `crates/routing`（新） | 路由解析与签名、回环网关（hyper + tokio + reqwest + rustls/ring）、探测 |
| `crates/agents` | pi 扩展、OMP 扩展与覆盖层、DSH patch 与 `mcpServers` 的投影；运行中切换模型 |
| `crates/app` | 设置对话框左侧导航与页面；聊天启动前路由、配置变化后重启；底部输入工具栏接入 LumaKit 独立模型／力度选择器，选择器向上展开 |
