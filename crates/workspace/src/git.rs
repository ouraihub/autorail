use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use thinkrail_core::{Branch, Change, ChangeStatus, Commit, Error, GitStatus, IoContext, Result};
use tracing::debug;

use crate::paths::bounded_read;

pub(crate) const MAX_OUTPUT: usize = 16 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub(crate) struct Output {
    pub success: bool,
    pub stdout: String,
}

/// Runs `git` without a shell, with hooks/fsmonitor disabled, `GIT_*` env removed, a 30 s timeout
/// and 16 MiB output limit. Exit codes in `allowed` are returned as `success: false`.
pub(crate) fn run(root: &Path, args: &[&str], allowed: &[i32]) -> Result<Output> {
    let started = Instant::now();
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args([
            "--no-optional-locks",
            "--literal-pathspecs",
            "-c",
            "core.hooksPath=",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .context("无法启动 git，请确认已安装 Git 并在 PATH 中")?;
    let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
        (Some(out), Some(err)) => (out, err),
        _ => return Err(Error::external("git 输出管道不可用")),
    };
    let out_rx = drain(stdout);
    let err_rx = drain(stderr);
    let status = loop {
        if let Some(status) = child.try_wait().context("等待 git")? {
            break status;
        }
        if started.elapsed() > TIMEOUT {
            terminate(&mut child);
            return Err(Error::timeout(format!(
                "git {} 超过 30 秒未完成",
                args.first().copied().unwrap_or("")
            )));
        }
        thread::sleep(POLL);
    };
    let stdout = received(&out_rx)?;
    let stderr = received(&err_rx)?;
    debug!(
        command = args.first().copied().unwrap_or(""),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "git"
    );
    if !status.success() && !status.code().is_some_and(|code| allowed.contains(&code)) {
        let detail = String::from_utf8_lossy(&stderr);
        let detail: String = detail.trim().chars().take(2048).collect();
        return Err(Error::external(format!(
            "git {} 失败：{detail}",
            args.first().copied().unwrap_or("")
        )));
    }
    let stdout = String::from_utf8(stdout).map_err(|_| Error::external("git 输出不是 UTF-8"))?;
    Ok(Output {
        success: status.success(),
        stdout,
    })
}

fn drain(reader: impl Read + Send + 'static) -> mpsc::Receiver<Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(bounded_read(reader, MAX_OUTPUT));
    });
    rx
}

fn received(rx: &mpsc::Receiver<Result<Vec<u8>>>) -> Result<Vec<u8>> {
    rx.recv_timeout(TIMEOUT)
        .map_err(|_| Error::timeout("读取 git 输出超时"))?
}

fn terminate(child: &mut std::process::Child) {
    if let Err(error) = child.kill() {
        debug!(%error, "git kill failed");
    }
    if let Err(error) = child.wait() {
        debug!(%error, "git reap failed");
    }
}

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
    let top = ok(dir, &["rev-parse", "--show-toplevel"])
        .map_err(|_| Error::invalid("所选文件夹不是 Git 仓库"))?;
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

fn has_head(root: &Path) -> Result<bool> {
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
    let output = ok(
        root,
        &[
            "log",
            "-z",
            &format!("-{limit}"),
            "--format=%H%x1f%s%x1f%an%x1f%ct",
            "--",
        ],
    )?;
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
    if committed.is_err() {
        let _ = fs::remove_dir_all(root.join(".git"));
    }
    committed
}
