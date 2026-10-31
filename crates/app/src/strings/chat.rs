//! Strings of the chat column.

pub const NEW_CHAT: &str = "新建聊天";
pub const CHAT: &str = "聊天";
pub const NO_WORKSPACE: &str = "先在左侧选择一个工作区。";
pub const NO_CHATS: &str = "这个工作区还没有聊天。点击右上角新建。";
pub const NO_PROFILE: &str = "还没有默认 Agent 档案。请先在设置中新建档案并设为默认。";
pub const PROFILE_MISSING: &str = "这个聊天的档案版本不存在或已禁用，只能查看，不能发送。";
pub const STARTING: &str = "正在启动 Agent…";
pub const CLOSED_HINT: &str = "Agent 进程未运行；发送消息时会恢复这个会话。";
pub const TRUNCATED: &str = "较早的消息未在此显示（Agent 仍保留完整历史）。";
pub const PRERELEASE: &str = "预发布";
pub const REVISION: &str = "版本";
pub const NO_MODEL: &str = "模型未知";
pub const PLAN: &str = "TODO 列表";

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
pub const OTHER: &str = "其他回答（可留空）";
