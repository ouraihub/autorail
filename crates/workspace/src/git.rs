//! Git domain queries and parsing; subprocess lifecycle lives in `process`.
use std::{
    fs,
    path::{Path, PathBuf},
};

use thinkrail_core::{Branch, Change, ChangeStatus, Commit, Error, GitStatus, IoContext, Result};
use tracing::warn;

mod process;
pub(crate) use process::{MAX_OUTPUT, run, run_with_hooks};

/// `git` that must succeed; returns stdout.
pub(crate) fn ok(root: &Path, args: &[&str]) -> Result<String> {
    Ok(run(root, args, &[])?.stdout)
}

/// Git rejects `\\?\` verbatim paths, which `fs::canonicalize` produces on Windows.
pub(crate) fn path_arg(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| Error::invalid("路径不是 UTF-8"))?;
    Ok(match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_owned(),
        _ => text.to_owned(),
    })
}

pub(crate) fn toplevel(dir: &Path) -> Result<PathBuf> {
    let output = run(dir, &["rev-parse", "--show-toplevel"], &[128])?;
    if !output.success {
        return Err(Error::invalid("所选文件夹不是 Git 仓库"));
    }
    let top = output.stdout;
    fs::canonicalize(top.trim_end_matches(['\r', '\n'])).context("解析仓库路径")
}

pub(crate) fn common_dir(root: &Path) -> Result<PathBuf> {
    let dir = ok(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    fs::canonicalize(dir.trim_end_matches(['\r', '\n'])).context("解析 Git 公共目录")
}

pub(crate) fn safe_ref(reference: &str) -> Result<()> {
    if reference.is_empty()
        || reference.starts_with('-')
        || reference
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(Error::invalid("Git 引用名无效"));
    }
    Ok(())
}

pub(crate) fn validate_branch(root: &Path, branch: &str) -> Result<()> {
    safe_ref(branch)?;
    ok(root, &["check-ref-format", &format!("refs/heads/{branch}")]).map(|_| ())
}

pub(crate) fn branch_exists(root: &Path, branch: &str) -> Result<bool> {
    Ok(run(
        root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
        &[1],
    )?
    .success)
}

pub(crate) fn current_branch(root: &Path) -> Result<String> {
    let symbolic = run(root, &["symbolic-ref", "--quiet", "--short", "HEAD"], &[1])?;
    if symbolic.success {
        return Ok(symbolic.stdout.trim_end().into());
    }
    let detached = run(root, &["rev-parse", "--short", "HEAD"], &[128])?;
    Ok(if detached.success {
        detached.stdout.trim_end().into()
    } else {
        String::new()
    })
}

pub(crate) fn default_branch(root: &Path) -> Result<String> {
    let remote = run(
        root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
        &[1],
    )?;
    if remote.success {
        return Ok(remote.stdout.trim_end().into());
    }
    for branch in ["main", "master"] {
        if branch_exists(root, branch)? {
            return Ok(branch.into());
        }
    }
    current_branch(root)
}

pub(crate) fn has_head(root: &Path) -> Result<bool> {
    Ok(run(
        root,
        &["rev-parse", "--verify", "--quiet", "HEAD"],
        &[1, 128],
    )?
    .success)
}

pub(crate) fn status(root: &Path) -> Result<GitStatus> {
    let output = ok(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut records = output.split('\0').filter(|s| !s.is_empty());
    let mut changes = Vec::new();
    while let Some(record) = records.next() {
        let bytes = record.as_bytes();
        if bytes.len() < 4 || !record.is_char_boundary(3) {
            return Err(Error::external("git status 输出格式异常"));
        }
        let (index, worktree) = (bytes[0], bytes[1]);
        let status = change_status(index, worktree);
        let original_path = if matches!(status, ChangeStatus::Renamed | ChangeStatus::Copied) {
            Some(
                records
                    .next()
                    .ok_or_else(|| Error::external("git status 缺少重命名来源"))?
                    .to_owned(),
            )
        } else {
            None
        };
        changes.push(Change {
            path: record[3..].to_owned(),
            original_path,
            status,
            staged: index != b' ' && index != b'?',
            additions: None,
            deletions: None,
        });
    }
    if has_head(root)? {
        apply_numstat(root, &mut changes)?;
    }
    let branch = current_branch(root)?;
    let upstream = run(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        &[128],
    )?;
    let (upstream, ahead, behind) = if upstream.success {
        let counts = ok(
            root,
            &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
        )?;
        let mut parts = counts
            .split_whitespace()
            .map(|n| n.parse::<u32>().unwrap_or(0));
        let behind = parts.next().unwrap_or(0);
        let ahead = parts.next().unwrap_or(0);
        (Some(upstream.stdout.trim_end().to_owned()), ahead, behind)
    } else {
        (None, 0, 0)
    };
    Ok(GitStatus {
        branch: (!branch.is_empty()).then_some(branch),
        upstream,
        ahead,
        behind,
        changes,
    })
}

fn change_status(index: u8, worktree: u8) -> ChangeStatus {
    let either = |c: u8| index == c || worktree == c;
    if index == b'?' {
        ChangeStatus::Untracked
    } else if either(b'U')
        || (index == b'A' && worktree == b'A')
        || (index == b'D' && worktree == b'D')
    {
        ChangeStatus::Conflicted
    } else if either(b'R') {
        ChangeStatus::Renamed
    } else if either(b'C') {
        ChangeStatus::Copied
    } else if either(b'D') {
        ChangeStatus::Deleted
    } else if either(b'A') {
        ChangeStatus::Added
    } else if either(b'T') {
        ChangeStatus::TypeChanged
    } else {
        ChangeStatus::Modified
    }
}

/// Fills `+additions -deletions` for tracked changes versus `HEAD` (binary files stay `None`).
fn apply_numstat(root: &Path, changes: &mut [Change]) -> Result<()> {
    let output = ok(root, &["diff", "--numstat", "-z", "-M", "HEAD", "--"])?;
    let mut fields = output.split('\0');
    while let Some(head) = fields.next() {
        if head.is_empty() {
            continue;
        }
        let mut parts = head.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        // A rename has an empty path here followed by `old\0new`.
        let path = if path.is_empty() {
            let _old = fields.next();
            fields.next().unwrap_or_default()
        } else {
            path
        };
        if let Some(change) = changes.iter_mut().find(|c| c.path == path) {
            change.additions = added.parse().ok();
            change.deletions = deleted.parse().ok();
        }
    }
    Ok(())
}

pub(crate) fn branches(root: &Path) -> Result<Vec<Branch>> {
    let current = current_branch(root)?;
    let local = ok(
        root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )?;
    Ok(local
        .lines()
        .map(|name| Branch {
            name: name.to_owned(),
            current: name == current,
        })
        .collect())
}

pub(crate) fn commits(root: &Path, limit: usize) -> Result<Vec<Commit>> {
    if limit == 0 || limit > 500 {
        return Err(Error::invalid("提交数量须为 1..500"));
    }
    if !has_head(root)? {
        return Ok(Vec::new());
    }
    let output = ok(root, &["log", "-z", &format!("-{limit}"), LOG_FORMAT, "--"])?;
    parse_log(&output)
}

/// The `--format` [`parse_log`] reads, with `-z`.
pub(crate) const LOG_FORMAT: &str = "--format=%H%x1f%s%x1f%an%x1f%ct";

pub(crate) fn parse_log(output: &str) -> Result<Vec<Commit>> {
    output
        .split('\0')
        .filter(|record| !record.trim().is_empty())
        .map(|record| {
            let fields: Vec<&str> = record.trim_start_matches('\n').split('\u{1f}').collect();
            let [hash, subject, author, seconds] = fields[..] else {
                return Err(Error::external("git log 输出格式异常"));
            };
            let seconds: u64 = seconds
                .trim()
                .parse()
                .map_err(|_| Error::external("git log 时间格式异常"))?;
            Ok(Commit {
                hash: hash.into(),
                subject: subject.into(),
                author: author.into(),
                timestamp: seconds * 1000,
            })
        })
        .collect()
}

pub(crate) fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// `git init -b main`, stage everything and an initial commit (empty when the folder is), so
/// HEAD exists and worktrees work. A failure removes the new `.git`. When the user has no Git
/// identity, the commit is signed `ThinkRail <thinkrail@localhost>` (original ThinkRail).
pub(crate) fn bootstrap(root: &Path) -> Result<()> {
    ok(root, &["init", "-q", "-b", "main"])?;
    let committed = (|| {
        ok(root, &["add", "-A"])?;
        let mut args: Vec<&str> = Vec::new();
        if run(root, &["config", "user.name"], &[1])?
            .stdout
            .trim()
            .is_empty()
        {
            args.extend(["-c", "user.name=ThinkRail"]);
        }
        if run(root, &["config", "user.email"], &[1])?
            .stdout
            .trim()
            .is_empty()
        {
            args.extend(["-c", "user.email=thinkrail@localhost"]);
        }
        args.extend([
            "commit",
            "-q",
            "--allow-empty",
            "--no-verify",
            "-m",
            "Initial commit",
        ]);
        ok(root, &args).map(|_| ())
    })();
    if committed.is_err()
        && let Err(error) = fs::remove_dir_all(root.join(".git"))
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!(code = "IO", io_kind = ?error.kind(), "workspace.git.bootstrap_cleanup_failed");
    }
    committed
}
