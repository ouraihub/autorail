use std::{fmt, io};

/// Stable, machine-readable classification of a failure.
///
/// UI and logs branch on the code; `Display` text is for humans only and is never parsed.
pub trait ErrorCode {
    fn code(&self) -> &'static str;
}

/// What kind of failure happened, so callers can react without string matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    NotFound,
    Invalid,
    Conflict,
    Busy,
    Corrupt,
    Forbidden,
    Timeout,
    External,
    BackendUnavailable,
    Protocol,
    SessionClosed,
    SessionBusy,
    InteractionExpired,
}

impl ErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotFound => "NOT_FOUND",
            Self::Invalid => "INVALID",
            Self::Conflict => "CONFLICT",
            Self::Busy => "BUSY",
            Self::Corrupt => "CORRUPT",
            Self::Forbidden => "FORBIDDEN",
            Self::Timeout => "TIMEOUT",
            Self::External => "EXTERNAL",
            Self::BackendUnavailable => "BACKEND_UNAVAILABLE",
            Self::Protocol => "PROTOCOL",
            Self::SessionClosed => "SESSION_CLOSED",
            Self::SessionBusy => "SESSION_BUSY",
            Self::InteractionExpired => "INTERACTION_EXPIRED",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::NotFound => "不存在",
            Self::Invalid => "参数无效",
            Self::Conflict => "已被修改，请重新读取",
            Self::Busy => "资源正被占用",
            Self::Corrupt => "数据已损坏，未做任何修改",
            Self::Forbidden => "不允许的操作",
            Self::Timeout => "操作超时",
            Self::External => "外部命令失败",
            Self::BackendUnavailable => "Agent 无法启动",
            Self::Protocol => "Agent 协议错误",
            Self::SessionClosed => "会话已关闭",
            Self::SessionBusy => "会话正在运行",
            Self::InteractionExpired => "该请求已不再等待回答",
        }
    }
}

/// The single error type at every service boundary (04 §2).
///
/// `context` is human detail and must never carry secrets, env values, prompt text or full stderr.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}：{context}", kind.label())]
    Domain { kind: ErrorKind, context: String },
    #[error("{context}：{source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
}

impl ErrorCode for Error {
    fn code(&self) -> &'static str {
        match self {
            Self::Domain { kind, .. } => kind.code(),
            Self::Io { .. } => "IO",
        }
    }
}

impl Error {
    pub fn new(kind: ErrorKind, context: impl fmt::Display) -> Self {
        Self::Domain { kind, context: context.to_string() }
    }

    /// `None` for I/O failures.
    pub fn kind(&self) -> Option<ErrorKind> {
        match self {
            Self::Domain { kind, .. } => Some(*kind),
            Self::Io { .. } => None,
        }
    }

    pub fn not_found(what: impl fmt::Display) -> Self { Self::new(ErrorKind::NotFound, what) }
    pub fn invalid(why: impl fmt::Display) -> Self { Self::new(ErrorKind::Invalid, why) }
    pub fn conflict(why: impl fmt::Display) -> Self { Self::new(ErrorKind::Conflict, why) }
    pub fn busy(why: impl fmt::Display) -> Self { Self::new(ErrorKind::Busy, why) }
    pub fn corrupt(why: impl fmt::Display) -> Self { Self::new(ErrorKind::Corrupt, why) }
    pub fn forbidden(why: impl fmt::Display) -> Self { Self::new(ErrorKind::Forbidden, why) }
    pub fn timeout(why: impl fmt::Display) -> Self { Self::new(ErrorKind::Timeout, why) }
    pub fn external(why: impl fmt::Display) -> Self { Self::new(ErrorKind::External, why) }
    pub fn protocol(why: impl fmt::Display) -> Self { Self::new(ErrorKind::Protocol, why) }
}

/// Attach human context to an I/O failure without losing the original error as `source`.
pub trait IoContext<T> {
    fn context(self, context: impl Into<String>) -> Result<T>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.map_err(|source| Error::Io { context: context.into(), source })
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_io_keeps_source() {
        let failed: io::Result<()> = Err(io::Error::from(io::ErrorKind::NotFound));
        let error = failed.context("读取配置").unwrap_err();
        assert_eq!(error.code(), "IO");
        assert!(std::error::Error::source(&error).is_some());
        let error = Error::new(ErrorKind::BackendUnavailable, "No default model selected");
        assert_eq!(error.code(), "BACKEND_UNAVAILABLE");
        assert_eq!(error.kind(), Some(ErrorKind::BackendUnavailable));
        assert_eq!(error.to_string(), "Agent 无法启动：No default model selected");
    }
}
