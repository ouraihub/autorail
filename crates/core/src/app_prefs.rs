//! Host-side preferences that are not domain settings (native-multi-agent 06 §4, 08): the log
//! page's switch, the log level and the appearance. Stored in `thinkrail.db` `app_prefs`.

use serde::{Deserialize, Serialize};

use crate::{AppearancePrefs, Result};

/// The levels the log page offers; `debug` shows the full call chain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[default]
    Info,
    Debug,
}

impl LogLevel {
    /// The `EnvFilter` directive.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppPrefs {
    /// 显示后台日志.
    pub logs_visible: bool,
    pub log_level: LogLevel,
    pub appearance: AppearancePrefs,
}

pub trait AppPrefService: Send + Sync {
    fn app_prefs(&self) -> Result<AppPrefs>;
    fn set_logs_visible(&self, visible: bool) -> Result<AppPrefs>;
    fn set_log_level(&self, level: LogLevel) -> Result<AppPrefs>;
    /// Replaces the whole appearance value; the stored value is what `app_prefs` returns next.
    fn set_appearance(&self, appearance: &AppearancePrefs) -> Result<AppPrefs>;
}
