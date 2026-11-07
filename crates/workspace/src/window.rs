//! Git facts and the host commit for plan-step work windows (agent-settings 08 §5.8), after the
//! original `git/git.ts` (`gitUncommittedPaths`, `listCommitsSince`, `gitCommitPaths`).

use std::{fs, path::Path};

use thinkrail_core::{Commit, Error, ErrorKind, Result};
use tracing::{info, warn};

use crate::git::{LOG_FORMAT, has_head, ok, parse_log, run, run_with_hooks, safe_ref};

fn nul_fields(output: &str) -> impl Iterator<Item = &str> {
    output.split('\0').filter(|field| !field.is_empty())
}

/// Tracked paths that differ from `HEAD` (both sides of a rename) plus untracked, not ignored
/// files; sorted, no duplicates. Without a `HEAD` every tracked and untracked file counts.
pub(crate) fn uncommitted_paths(root: &Path) -> Result<Vec<String>> {
    let tracked = if has_head(root)? {
        ok(
            root,
            &[
                "diff",
                "--name-only",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "HEAD",
                "--",
            ],
        )?
    } else {
        ok(root, &["ls-files", "-z", "--cached"])?
    };
    let untracked = ok(root, &["ls-files", "-z", "--others", "--exclude-standard"])?;
    let mut paths: Vec<String> = nul_fields(&tracked)
        .chain(nul_fields(&untracked))
        .map(str::to_owned)
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// The full `HEAD` sha; `None` before the first commit.
pub(crate) fn head(root: &Path) -> Result<Option<String>> {
    let output = run(
        root,
        &["rev-parse", "--verify", "--quiet", "HEAD"],
        &[1, 128],
    )?;
    Ok(output
        .success
        .then(|| output.stdout.trim().to_owned())
        .filter(|sha| !sha.is_empty()))
}

/// `base..HEAD`, oldest first, at most `limit` (the oldest ones). Empty without a `HEAD`.
pub(crate) fn log_range(root: &Path, base: &str, limit: usize) -> Result<Vec<Commit>> {
    if limit == 0 || limit > 500 {
        return Err(Error::invalid("提交数量须为 1..500"));
    }
    safe_ref(base)?;
    if !has_head(root)? {
        return Ok(Vec::new());
    }
    let range = format!("{base}..HEAD");
    let output = ok(root, &["log", "-z", "--reverse", LOG_FORMAT, &range, "--"])?;
    let mut commits = parse_log(&output)?;
    commits.truncate(limit);
    Ok(commits)
}

/// Commits exactly `paths` with `subject`, with the user's identity and hooks. `None` when there
/// is nothing staged for them, a merge is unresolved, or the commit is refused (a hook): the
/// index is then put back as it was. A command that failed although its commit landed is
/// recognised by parent, message and paths (the original `isOurCommit`).
pub(crate) fn commit_paths(root: &Path, subject: &str, paths: &[String]) -> Result<Option<String>> {
    let subject = subject.lines().next().unwrap_or("").trim();
    if paths.is_empty() || subject.is_empty() {
        return Ok(None);
    }
    if !ok(root, &["ls-files", "-u"])?.trim().is_empty() {
        info!("host commit skipped: unmerged paths");
        return Ok(None);
    }
    let index = index_path(root)?;
    let saved = match fs::read(&index) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(Error::Io {
                context: "读取 git 索引".into(),
                source: error,
            });
        }
    };
    let restore = || {
        let restored = match &saved {
            Some(bytes) => fs::write(&index, bytes),
            None => fs::remove_file(&index).or_else(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error)
                }
            }),
        };
        if let Err(error) = restored {
            warn!(%error, "git index restore failed");
        }
    };
    let before = head(root)?;
    let with_paths = |command: &[&'static str]| -> Vec<&str> {
        let mut args: Vec<&str> = command.to_vec();
        args.push("--");
        args.extend(paths.iter().map(String::as_str));
        args
    };
    // A path that vanished meanwhile fails `add`; like the original, that is "nothing committed".
    if let Err(error) = ok(root, &with_paths(&["add", "-A"])) {
        restore();
        if error.kind() == Some(ErrorKind::Timeout) {
            return Err(error);
        }
        info!(%error, "host commit skipped: staging failed");
        return Ok(None);
    }
    match run(root, &with_paths(&["diff", "--cached", "--quiet"]), &[1]) {
        Ok(output) if output.success => {
            // Nothing staged for these paths: the delta was undone meanwhile.
            restore();
            return Ok(None);
        }
        Ok(_) => {}
        Err(error) => {
            restore();
            return Err(error);
        }
    }
    let mut args = vec!["commit", "-q", "-m", subject];
    args.extend(with_paths(&[]));
    let committed = run_with_hooks(root, &args, &[]);
    let after = head(root)?;
    if committed.is_ok() {
        return Ok(after);
    }
    if after != before
        && let Some(sha) = &after
        && is_our_commit(root, sha, before.as_deref(), subject, paths)?
    {
        return Ok(after);
    }
    if let Err(error) = &committed {
        info!(%error, "host commit refused");
    }
    restore();
    Ok(None)
}

fn index_path(root: &Path) -> Result<std::path::PathBuf> {
    let path = ok(
        root,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?;
    let path = path.trim();
    if path.is_empty() {
        return Err(Error::external("git 没有返回索引路径"));
    }
    Ok(std::path::PathBuf::from(path))
}

fn is_our_commit(
    root: &Path,
    sha: &str,
    before: Option<&str>,
    subject: &str,
    paths: &[String],
) -> Result<bool> {
    let parent = run(
        root,
        &["rev-parse", "--verify", "--quiet", &format!("{sha}^")],
        &[1, 128],
    )?;
    let parent = parent.success.then(|| parent.stdout.trim().to_owned());
    if parent.as_deref() != before {
        return Ok(false);
    }
    let message = ok(root, &["log", "-1", "--format=%B", sha, "--"])?;
    if message.trim() != subject {
        return Ok(false);
    }
    let mut args = vec!["diff-tree", "--quiet", "-r", "--root", sha, "--"];
    args.extend(paths.iter().map(String::as_str));
    // `--quiet` exits 1 when the commit touches the paths.
    Ok(!run(root, &args, &[1])?.success)
}
