//! 后台日志设置页文案（native-multi-agent 06 §5）。

pub const TITLE: &str = "后台日志";
pub const SHOW: &str = "显示后台日志";
pub const SHOW_HINT: &str = "日志只在此页打开时读取，不影响后台写入。";
pub const LEVEL: &str = "日志级别";
pub const LEVEL_HINT: &str = "调试级别记录每次操作的完整调用链，排查问题时打开；立即生效。";
pub const LEVEL_PINNED: &str = "已由环境变量 THINKRAIL_LOG 指定，此处设置不生效。";
pub const INFO: &str = "信息";
pub const DEBUG: &str = "调试";
pub const ONLY_PROBLEMS: &str = "只看问题";
pub const REFRESH: &str = "刷新";
pub const OPEN_DIR: &str = "打开日志目录";
pub const COPY: &str = "复制当前日志";
pub const COPY_TRACE: &str = "复制 Trace ID";
pub const COPY_TRACE_HINT: &str = "复制此条日志的 Trace ID";
pub const COPIED: &str = "已复制当前显示的日志";
pub const FILTER: &str = "筛选：Trace ID、op、chat、模块名";
pub const EMPTY: &str = "还没有日志。";
pub const NO_PROBLEMS: &str = "没有警告或错误。";
pub const NO_MATCHES: &str = "没有符合筛选的日志。";
pub const READ_FAILED: &str = "日志读取失败：";

pub fn recent(count: usize) -> String {
    format!("最近 {count} 条 · 每 2 秒刷新")
}
