use std::{collections::HashSet, path::Path};

use thinkrail_core::{ChatBinding, EnvSource, Error, Launch, ProfileId, ProfileInput, Result};
use uuid::{Uuid, Variant};

/// Largest revision a JSON consumer can represent exactly (`Number.MAX_SAFE_INTEGER`).
pub(crate) const MAX_REVISION: u64 = 9_007_199_254_740_991;
const MAX_PATH: usize = 32_767;
const MAX_ENTRIES: usize = 256;
const RESERVED_ENV: [&str; 3] = ["HOME", "USERPROFILE", "PATH"];
const RESERVED_ENV_PREFIX: &str = "THINKRAIL_";

pub(crate) fn input(input: &ProfileInput) -> Result<()> {
    text(&input.label, "档案名称", 200)?;
    if let Some(root) = &input.config_root {
        absolute_path(root, "配置目录")?;
    }
    let Launch::External { executable, args } = &input.launch;
    absolute_path(executable, "可执行文件")?;
    if args.len() > MAX_ENTRIES {
        return Err(Error::invalid(format!("启动参数最多 {MAX_ENTRIES} 个")));
    }
    if args.iter().any(|arg| arg.encode_utf16().count() > MAX_PATH || has_control(arg)) {
        return Err(Error::invalid("启动参数不能包含控制字符，单个参数最长 32767 个字符"));
    }
    if input.env_refs.len() > MAX_ENTRIES {
        return Err(Error::invalid(format!("环境变量引用最多 {MAX_ENTRIES} 个")));
    }
    let mut names = HashSet::with_capacity(input.env_refs.len());
    for entry in &input.env_refs {
        text(&entry.name, "环境变量名", 256)?;
        let canonical = entry.name.to_ascii_uppercase();
        if !env_name(&entry.name)
            || RESERVED_ENV.contains(&canonical.as_str())
            || canonical.starts_with(RESERVED_ENV_PREFIX)
            || !names.insert(canonical)
        {
            return Err(Error::invalid("环境变量名无效、重复，或是保留名（HOME、USERPROFILE、PATH、THINKRAIL_*）"));
        }
        text(&entry.reference, "环境变量引用", 512)?;
        let valid = match entry.source {
            EnvSource::Launch => env_name(&entry.reference),
        };
        if !valid {
            return Err(Error::invalid("环境变量只能引用启动环境中的变量名，不能直接填写值"));
        }
    }
    Ok(())
}

pub(crate) fn binding(binding: &ChatBinding) -> Result<()> {
    identifier(binding.chat_id.as_str(), "聊天 id")?;
    identifier(binding.workspace_id.as_str(), "工作区 id")?;
    profile_id(&binding.profile_id)?;
    revision(binding.config_revision)?;
    text(binding.native_session_key.as_str(), "原生会话 locator", MAX_PATH)
}

pub(crate) fn profile_id(id: &ProfileId) -> Result<()> {
    let id = id.as_str();
    let valid = id.len() == 36
        && Uuid::try_parse(id)
            .is_ok_and(|uuid| matches!(uuid.get_version_num(), 1..=8) && uuid.get_variant() == Variant::RFC4122);
    if valid { Ok(()) } else { Err(Error::invalid("档案 id 必须是 UUID")) }
}

pub(crate) fn revision(revision: u64) -> Result<()> {
    if (1..=MAX_REVISION).contains(&revision) {
        Ok(())
    } else {
        Err(Error::invalid("配置版本号必须是 1 到 2^53-1 之间的整数"))
    }
}

fn has_control(text: &str) -> bool {
    text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

fn text(value: &str, field: &str, maximum: usize) -> Result<()> {
    if value.is_empty() || value.trim() != value || has_control(value) || value.encode_utf16().count() > maximum {
        return Err(Error::invalid(format!(
            "{field}不能为空、首尾不能有空白、不能包含控制字符，最长 {maximum} 个字符"
        )));
    }
    Ok(())
}

fn identifier(value: &str, field: &str) -> Result<()> {
    text(value, field, 256)?;
    let bytes = value.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() || !bytes.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
        return Err(Error::invalid(format!("{field}只能包含字母、数字、_ 和 -，并以字母或数字开头")));
    }
    Ok(())
}

fn env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(c) if c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn absolute_path(path: &Path, field: &str) -> Result<()> {
    let Some(value) = path.to_str() else {
        return Err(Error::invalid(format!("{field}必须是 UTF-8 路径")));
    };
    text(value, field, MAX_PATH)?;
    let mut parts = value.split(['/', '\\']);
    let device = parts.next() == Some("") && parts.next() == Some("") && matches!(parts.next(), Some("?" | "."));
    if device || !path.is_absolute() || value.split(['/', '\\']).any(|part| matches!(part, "." | "..")) {
        return Err(Error::invalid(format!("{field}必须是绝对路径，不能包含 . 或 ..，也不能使用设备命名空间")));
    }
    Ok(())
}
