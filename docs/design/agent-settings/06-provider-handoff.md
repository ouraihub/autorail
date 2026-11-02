# Provider UI 后续实现交接记录

状态：已被取代。模型服务页改为 Aether 布局：立即保存，挑选对话框加 `merge_picked`，模型设置对话框编辑副本。现行设计见 04 §7.6–§7.7，验收见 05 T12。下文是改版前的记录。文中“不得注册 `#[cfg(test)] mod tests;`、不恢复 `src/**/tests.rs`”的测试位置规则也已废止，现行规则见 `SPEC.md` 的 Tests 与 [AI 工程规范](../../AI工程规范.md) §8。

## 已完成且保留

- Provider 页面已经存在：提供方列表、编辑 draft、地址预览、默认协议、接入方式、请求头、模型行、密钥管理、异步保存和 revision 检查。
- `ProviderService`、`ProviderProbe` 和 `ProviderModel` 的契约已在 `crates/core` 与 `crates/routing` 定义。
- Provider feature test 已从生产目录迁移到 `crates/app/tests/providers.rs`；生产模块不得重新注册 `#[cfg(test)] mod tests;`。
- 本次交接只提交文档与测试目录分离，不提交未完成的 Picker 或模型编辑实现。

## 已确认的问题

1. **同步模型会直接追加**：当前 `models.rs::ProvidersPage::discover` 在 probe 返回后直接向 `editor.models` 添加模型。它没有“选择 → 确认”阶段，无法可靠支持取消、部分选择和重复同步。
2. **Picker 状态边界未完成**：`ProviderEditor` 是 draft owner；Picker 不能直接持有或修改其私有字段。选择结果应先保存在 Picker 内，确认时通过 coordinator 的窄方法一次性合并。
3. **模型详情仍是行内展开**：现有 `render_model` 同时承担摘要、开关、检测和详情编辑。后续需要改为独立编辑层；取消必须不改变 draft，确认才原子替换模型。
4. **保存链路需要保留两阶段语义**：新建 Provider 必须先 `create`，再 `add_key`。第二步失败时不能丢失编辑状态，也不能重复创建 Provider。
5. **协议约束必须在 UI 与 service 两层一致**：默认协议必须对应已填写地址；模型协议必须对应已填写地址；`parse_url` / `parse_headers` 只做 UI 草稿校验，core/settings-store 仍是最终边界。
6. **事件与快照必须一致**：Provider、密钥和模型成功写入后发 `AppEvent::Routing`；异步回调只能接受当前 operation generation，不能用迟到结果覆盖新编辑。

## 目标状态

```text
ProviderEditor (唯一 draft owner)
  ├─ endpoint/header/key draft
  ├─ models: Vec<ModelDraft>
  └─ probe result -> ModelPicker (临时选择状态)
                         ├─ cancel -> 丢弃选择
                         └─ confirm -> ProvidersPage::apply_discovered_models
                                             ├─ existing id: 保留手工字段，只补非空 context_window
                                             └─ new id: 建立 disabled ModelDraft

Model row
  ├─ summary + enabled switch
  ├─ edit -> ModelEditor dialog (编辑副本)
  ├─ check -> ProviderProbe::check
  └─ delete -> 删除 draft 行
```

## 后续实现顺序

1. 先加纯状态合并函数：输入已有模型、探测结果和选择集合，输出新的模型 draft；覆盖重复 id、取消、保留手工字段、上下文窗口补全。
2. 为合并函数添加 crate-level integration/contract test；先不接 GPUI dialog。
3. 实现 `ModelPicker`，只负责过滤、勾选和返回选择集合；不访问 `ProviderEditor` 私有字段。
4. 在 `ProvidersPage` 实现 `apply_discovered_models`，由 coordinator 创建/替换 `ModelFields`，再接 dialog 的确认回调。
5. 将 `render_model` 拆成行摘要和 `ModelEditor` dialog。编辑副本，确认按 model id 原子替换；推理关闭时清空 efforts。
6. 修复并验证 create → add_key 两阶段保存、revision 冲突、Routing 事件和迟到回调。
7. 做 focused test，再做 workspace build/test/clippy；最后做真实窗口验证。真实中转验证需要用户同意，不能用截图替代状态测试。

## 不允许的捷径

- 不把 Picker 直接挂到 `ProviderEditor` 私有字段上。
- 不在探测回调里自动启用或直接追加全部模型。
- 不为测试把生产字段改成 `pub`，也不恢复 `src/**/tests.rs`。
- 不让新建流程在 add_key 失败后自动重试 create。
- 不把密钥明文写入日志、错误消息、快照或测试输出。
- 不以“能编译”替代窗口路径验证。

## 验收证据

实现完成后必须在 `docs/design/agent-settings/05-tasks.md` 的 T12 写入实际命令、测试结果和窗口验证结果。未执行的验证必须明确标记为“未验证”，不得写成完成。
