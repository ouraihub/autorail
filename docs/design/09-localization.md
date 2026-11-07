# Native 多语言设计

状态：设计已确定，待实施。

## 1. 目标与边界

Native 需要把用户界面从当前集中式中文常量迁移到可维护的多语言 catalog。方案对齐 GPUI Kit 0.9.0 的应用级 i18n 建议，并吸收 ThinkRail Web、uutils/coreutils 与 Rox 的工程约束。

本设计只覆盖 Native UI 文案：设置页、按钮、状态、错误提示、TODO 状态、评审卡片、引擎安装提示和菜单。

以下内容**不经过 UI locale 翻译**：

- Agent 回复；
- Host tool 结果；
- MCP/IPC 协议字段；
- Git、引擎和外部命令输出；
- skills；
- Reviewer system prompt、fact package、fix package 和 `request_review` 模型提示；
- 日志与调试文本。

理由：模型协议文本需要稳定，Agent-facing 文案保持英文；用户界面文案才随语言切换。

## 2. 技术选择

### 2.1 选型

采用：

- `rust-i18n`；
- YAML catalog；
- 英文作为 source locale；
- 首期内置 `en` 与 `zh-CN`；
- 后续可增加 `ja`、`es`、`ru`。

GPUI Kit 0.9.0 的官方 i18n 指南直接推荐应用 crate 使用 `rust-i18n` 和应用自己的 `locales/ui.yml`。Native 不引入 i18next，也不复制 GPUI Component 的内部 catalog。

### 2.2 不选 Fluent 的原因

Fluent（`fluent-bundle` / `i18n-embed`）在复杂复数、选择器、外部翻译包和大型翻译团队场景更强，uutils/coreutils 与 Rox 已验证这条路线。

Native 当前文案规模和消息复杂度不需要这套额外运行时和 build 复杂度。若后续出现复杂复数、性别/选择器、外挂语言包或 ICU 级格式化需求，再评估迁移到 Fluent；不提前引入未使用的复杂度。

### 2.3 不自研 JSON catalog

当前 `crates/app/src/strings/*.rs` 仅集中存放中文常量，没有 locale、fallback、参数一致性或 key parity 机制。继续扩展它会形成第二套 i18n 体系。

## 3. Catalog 布局

```text
crates/app/
  locales/
    ui.yml
  src/
    i18n.rs
```

Catalog 以完整语义 key 组织，不以英文显示文本作为 key：

```yaml
_version: 2

app:
  chat:
    review:
      reviewing:
        en: Reviewing…
        zh-CN: 正在评审…
      approved:
        en: Reviewed
        zh-CN: 已检查
      changes:
        en: Changes required
        zh-CN: 需要修复
      failed:
        en: Review failed
        zh-CN: 评审失败
```

推荐 key 命名：

```text
app.chat.review.reviewing
app.chat.review.approved
app.chat.review.changes
app.chat.review.failed
app.chat.review.retry
app.chat.review.send_fix
app.settings.models.review.title
app.settings.models.review.agent_enabled
app.engine.missing
app.engine.start_failed
```

组件库的 key 使用 `gpui_component` namespace；Native 自己的 key 使用 `app` namespace。应用不得读取组件内部 key 作为自己的文案来源。

## 4. Locale 状态与存储

### 4.1 偏好字段

扩展现有 `AppPrefs`：

```rust
pub struct AppPrefs {
    pub locale: String,
    // appearance/log 等现有字段
}
```

`locale` 的值：

```text
system
 en
 zh-CN
```

存储复用现有 `AppPrefService` 和 `app_prefs` key/value 表；不创建语言专用表，不把 locale 写入聊天、工作区或项目数据。

### 4.2 语言选择

启动时：

```text
显式 locale
→ system locale
→ en
```

系统语言匹配顺序：

1. 完整 BCP-47 tag；
2. 基础语言；
3. 同 script 的可用 locale；
4. `en`。

第一期至少保证 `en` 与 `zh-CN`。未知显式 locale 回退 `en`，并保留用户原始偏好以便文件补回后继续使用。

### 4.3 GPUI 刷新

`set_locale()` 是全局 locale 变化，不自动进入 GPUI 变更追踪。切换后必须：

```rust
rust_i18n::set_locale(locale);
cx.refresh_windows();
```

同时重建依赖语言的缓存：页面标题、菜单、picker 选项、ChatPanel 状态文案。element id、持久化值、逻辑 key 不得使用翻译后的文本。

## 5. 文案调用规则

UI 代码只通过 catalog key 取得用户可见文本：

```rust
rust_i18n::t!("app.chat.review.reviewing")
```

带变量时把完整句子放在 catalog，不能在 Rust 中拼接多个翻译片段：

```yaml
app.chat.review.open_findings:
  en: "{count} findings require attention"
  zh-CN: "需要修复 · {count} 条"
```

```rust
rust_i18n::t!("app.chat.review.open_findings", count = count)
```

变量名必须在 source locale 和所有翻译中一致；翻译可以改变词序。

### 5.1 不可翻译的身份字符串

以下内容保持稳定，不进入 UI 文案翻译：

- ChatId、PlanItemId、ReviewId；
- element id；
- host tool 名称；
- backend 名称和协议名；
- Git ref、文件路径、模型 id、provider id；
- 序列化字段和命令名。

显示 label 与逻辑 identity 分离；不能通过比较显示文本判断状态。

## 6. Fallback 与质量规则

- `en` 是 source of truth，所有 key 必须存在英文值；
- 缺少翻译的 key 回退 `en`；
- 未知 key 开发模式显示明显的 key marker 并记录一次 warning，不能静默显示空文本；
- 非字符串、空字符串和非法 catalog 条目不覆盖英文；
- 参数缺失或多余参数在测试中失败；
- 不允许通过 `format!` 拼接跨语言句子；
- 不允许使用显示文本作为逻辑分支、HashMap key 或 element id。

## 7. 语言文件与发布

第一期语言随二进制内置，保证离线运行和英文 fallback。用户自定义语言文件不是第一期范围；未来若实现，文件名遵循 BCP-47：

```text
<user-config>/locales/de.json
<user-config>/locales/pt-BR.json
```

自定义 catalog 必须：

- 限制文件大小；
- 校验 JSON/YAML 对象结构；
- 拒绝路径穿越和 dotfile；
- 只接受合法 BCP-47 文件名；
- overlay 在内置 catalog 之后应用；
- 缺失 key 回退英文；
- 删除 overlay 后恢复内置/fallback，不残留进程状态。

这部分必须单独设计，不能先把任意用户文件加载进全局 locale。

## 8. 测试与 CI

新增 `crates/app/src/i18n.rs` 测试及 locale parity 检查：

- `en` 包含所有 source key；
- 每个目标 locale 的 key 不得拼错；
- 缺少翻译回退英文；
- locale 选择覆盖 `system`、完整 tag、基础语言、script、未知值；
- 变量名一致；
- catalog 解析错误可观察；
- 切换 locale 后 GPUI 窗口刷新。

开发模式增加 pseudo-locale 选项：

```text
THINKRAIL_PSEUDO_LOCALE=1
```

pseudo-locale 给文本加 marker 并适度扩展长度，用于发现布局溢出、截断和固定宽度按钮问题。

日期、数字、金额和单位不由 locale setter 自动格式化；需要时使用 locale-aware formatter，避免 `format!` 假定英文小数点或分组规则。

## 9. 迁移顺序

1. 添加 `rust-i18n`、`crates/app/locales/ui.yml` 和 `i18n.rs`；
2. 迁移 `strings/chat.rs`、`settings_*`、`agent_guide`、`shell`、`terminal` 等 UI 文案；
3. 将新评审文案迁移为 `app.chat.review.*`；
4. 扩展 `AppPrefs`、`AppPrefService`、SQLite key/value 读写和语言设置页；
5. 在应用启动的 GPUI 初始化附近设置 locale；
6. locale 切换后刷新窗口，并重建缓存 label；
7. 删除已经没有调用方的旧中文常量；
8. 添加 parity、fallback、变量和 pseudo-locale 测试；
9. 做英文、简体中文和伪语言实窗验证；
10. 只有在复杂消息需求出现后，才评估 Fluent 迁移。

## 10. 验收

- 新鲜数据目录默认显示英文或系统语言，不因缺少翻译崩溃；
- 显式选择 `zh-CN` 后所有已迁移 UI 文案切换，窗口立即刷新；
- 缺失 key 显示英文 fallback，并在开发模式可定位；
- 评审状态、引擎未安装提示、错误提示和设置页均可翻译；
- Agent、tool、prompt、Git、日志文本保持稳定原文；
- English catalog parity、变量检查和 pseudo-locale 检查进入 CI；
- 语言切换不改变 element id、聊天 id、计划 id、工具名和持久化格式。
