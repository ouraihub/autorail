use std::{fs, path::Path};

use tempfile::TempDir;

use crate::{git, window};

fn sh(root: &Path, args: &[&str]) -> String {
    git::ok(root, args).unwrap()
}

fn repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    sh(root, &["init", "-q", "-b", "main"]);
    sh(root, &["config", "user.name", "t"]);
    sh(root, &["config", "user.email", "t@t"]);
    temp
}

fn commit_all(root: &Path, message: &str) -> String {
    sh(root, &["add", "-A"]);
    sh(root, &["commit", "-q", "-m", message]);
    window::head(root).unwrap().unwrap()
}

fn files_of(root: &Path, sha: &str) -> Vec<String> {
    sh(root, &["show", "--name-only", "--format=", sha])
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn facts_cover_unborn_renames_untracked_and_ranges() {
    let temp = repo();
    let root = temp.path();
    fs::write(root.join("a.txt"), "a").unwrap();
    assert_eq!(window::head(root).unwrap(), None);
    assert_eq!(window::uncommitted_paths(root).unwrap(), ["a.txt"]);
    assert!(window::log_range(root, "HEAD", 10).unwrap().is_empty());

    let first = commit_all(root, "one");
    assert!(window::uncommitted_paths(root).unwrap().is_empty());
    sh(root, &["mv", "a.txt", "b.txt"]);
    fs::write(root.join("中文 名.txt"), "x").unwrap();
    fs::write(root.join(".gitignore"), "ignored\n").unwrap();
    fs::write(root.join("ignored"), "x").unwrap();
    assert_eq!(
        window::uncommitted_paths(root).unwrap(),
        [".gitignore", "a.txt", "b.txt", "中文 名.txt"]
    );

    let second = commit_all(root, "two");
    fs::write(root.join("c"), "c").unwrap();
    let third = commit_all(root, "three");
    let range = window::log_range(root, &first, 10).unwrap();
    let shas: Vec<&str> = range.iter().map(|c| c.hash.as_str()).collect();
    assert_eq!(shas, [second.as_str(), third.as_str()], "oldest first");
    assert_eq!(range[0].subject, "two");
    let capped = window::log_range(root, &first, 1).unwrap();
    assert_eq!(capped[0].hash, second, "the cap keeps the oldest");
    assert!(window::log_range(root, "-x", 1).is_err());
}

#[test]
fn commit_paths_commits_only_the_delta_and_keeps_the_rest_of_the_index() {
    let temp = repo();
    let root = temp.path();
    fs::write(root.join("user.txt"), "u").unwrap();
    fs::write(root.join("gone.txt"), "g").unwrap();
    let base = commit_all(root, "base");
    // The user's own staged and unstaged work.
    fs::write(root.join("user.txt"), "u2").unwrap();
    sh(root, &["add", "user.txt"]);
    fs::write(root.join("mine.txt"), "m").unwrap();
    // The step's delta: a new file and a deletion.
    fs::write(root.join("step.txt"), "s").unwrap();
    fs::remove_file(root.join("gone.txt")).unwrap();
    let delta = vec!["gone.txt".to_owned(), "step.txt".to_owned()];

    let sha = window::commit_paths(root, "Step done\nsecond line", &delta)
        .unwrap()
        .unwrap();
    let mut files = files_of(root, &sha);
    files.sort();
    assert_eq!(files, delta);
    assert_eq!(
        sh(root, &["log", "-1", "--format=%B", &sha]).trim(),
        "Step done"
    );
    assert_eq!(sh(root, &["rev-parse", &format!("{sha}^")]).trim(), base);
    assert_eq!(
        sh(root, &["diff", "--cached", "--name-only"]).trim(),
        "user.txt"
    );
    assert_eq!(
        window::uncommitted_paths(root).unwrap(),
        ["mine.txt", "user.txt"]
    );

    // Nothing left for those paths: no commit.
    assert_eq!(window::commit_paths(root, "again", &delta).unwrap(), None);
    assert_eq!(
        window::commit_paths(root, "  ", &["mine.txt".into()]).unwrap(),
        None
    );
    assert_eq!(window::head(root).unwrap().unwrap(), sha);
}

#[test]
fn a_refusing_hook_commits_nothing_and_restores_the_index() {
    let temp = repo();
    let root = temp.path();
    fs::write(root.join("base.txt"), "b").unwrap();
    commit_all(root, "base");
    let hooks = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(hooks.join("pre-commit"), "#!/bin/sh\necho no >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hooks.join("pre-commit"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(root.join("step.txt"), "s").unwrap();
    let before = window::head(root).unwrap();
    assert_eq!(
        window::commit_paths(root, "Step", &["step.txt".into()]).unwrap(),
        None
    );
    assert_eq!(window::head(root).unwrap(), before);
    assert!(
        sh(root, &["diff", "--cached", "--name-only"])
            .trim()
            .is_empty(),
        "the delta is unstaged again"
    );
    // A passing hook runs and the commit lands.
    fs::write(
        hooks.join("pre-commit"),
        "#!/bin/sh\ntouch hook-ran\nexit 0\n",
    )
    .unwrap();
    let sha = window::commit_paths(root, "Step", &["step.txt".into()])
        .unwrap()
        .unwrap();
    assert!(root.join("hook-ran").exists());
    assert_eq!(files_of(root, &sha), ["step.txt"]);
}

#[test]
fn git_failure_does_not_expose_stderr_arguments_or_paths() {
    let temp = repo();
    let error = git::ok(
        temp.path(),
        &["rev-parse", "--verify", "private-input-sentinel"],
    )
    .unwrap_err();
    assert_eq!(error.kind(), Some(thinkrail_core::ErrorKind::External));
    let message = error.to_string();
    assert!(!message.contains("private-input-sentinel"));
    assert!(!message.contains(temp.path().to_str().unwrap()));
}

#[test]
fn a_failed_index_restore_is_not_reported_as_a_skipped_commit() {
    let temp = repo();
    let root = temp.path();
    fs::write(root.join("base.txt"), "base").unwrap();
    commit_all(root, "base");
    let hooks = root.join(".git/hooks");
    fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    fs::write(
        &hook,
        "#!/bin/sh\nrm -f .git/index\nmkdir .git/index\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(root.join("step.txt"), "step").unwrap();
    let error = window::commit_paths(root, "Step", &["step.txt".into()]).unwrap_err();
    assert!(matches!(error, thinkrail_core::Error::Io { .. }));
}
