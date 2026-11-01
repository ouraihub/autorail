//! Strings of the default-Agent guidance (agent-settings 07 §5.1).

pub const NONE_INSTALLED: &str = "还没有安装 Agent。装好一个后，就能让 Agent 起草规格。";
pub const INSTALL: &str = "去安装";
pub const SET_DEFAULT: &str = "去设置";
pub const OPEN_CODING: &str = "去编码搭档";

pub fn no_default(name: &str) -> String {
    format!("已安装 {name}，还没有设为默认 Agent。")
}

pub fn missing(name: &str) -> String {
    format!("{name} 未找到，请在编码搭档中重新安装或换一个。")
}
