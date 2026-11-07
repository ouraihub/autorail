//! Strings of the chat column.

pub const NEW_CHAT: &str = "新建聊天";
pub const CHAT: &str = "聊天";
pub const NO_WORKSPACE: &str = "先在左侧选择一个工作区。";
pub const NO_CHATS: &str = "这个工作区还没有聊天。点击右上角新建。";
pub const STARTING: &str = "正在启动 Agent…";
pub const CLOSED_HINT: &str = "Agent 进程未运行；发送消息时会恢复这个会话。";
pub const TRUNCATED: &str = "较早的消息未在此显示（Agent 仍保留完整历史）。";
pub const PRERELEASE: &str = "预发布";
pub const PLAN: &str = "TODO 列表";
pub const REVIEWING: &str = "正在评审…";
pub const REVIEW_APPROVED: &str = "已检查";
pub const REVIEW_CHANGES: &str = "需要修复";
pub const REVIEW_FAILED: &str = "评审失败";

pub const PHASE_IDLE: &str = "空闲";
pub const PHASE_RUNNING: &str = "运行中";
pub const PHASE_WAITING: &str = "等待回答";
pub const PHASE_CLOSED: &str = "未运行";

pub const THINKING: &str = "思考";
pub const CHARS: &str = "字";
pub const TOOL_INPUT: &str = "输入";
pub const TOOL_OUTPUT: &str = "输出";
pub const IMAGE: &str = "图片";

pub const PLACEHOLDER: &str = "输入消息；Enter 发送，Shift+Enter 换行";
pub const SEND: &str = "发送";
pub const STOP: &str = "停止";

pub const YES: &str = "是";
pub const NO: &str = "否";
pub const SUBMIT: &str = "提交";
pub const CANCEL: &str = "取消";
pub const PERMISSION: &str = "请求运行工具";
pub const QUESTION: &str = "Agent 提问";
pub const OTHER: &str = "其他（自行填写）";

pub const CMD_COMPACT: &str = "手动压缩上下文 · 可选指令";
pub const CMD_NAME: &str = "重命名此会话 · 需要填写标题";
pub const CMD_MODEL: &str = "切换模型和力度 · 例如 /model opus high";
pub const CMD_BUILTIN: &str = "内置";
pub const NAME_EMPTY: &str = "请在 /name 后填写标题";
pub const MODEL_USAGE: &str = "用法：/model <模型名> [力度]，例如 /model opus high";
pub const MODEL_UNKNOWN: &str = "当前模型不在可选列表中，无法只改力度";

pub fn builtin_source(backend: &str) -> String {
    format!("{backend} / {CMD_BUILTIN}")
}

pub fn model_none(query: &str) -> String {
    format!("没有匹配“{query}”的模型")
}

/// `candidates` are already `提供方 · 模型` labels.
pub fn model_many(candidates: &[String]) -> String {
    format!("匹配到多个模型：{}，请写得更具体", candidates.join("、"))
}

pub fn effort_unsupported(effort: &str) -> String {
    format!("这个模型不支持力度“{effort}”")
}
