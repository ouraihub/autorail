use std::{fs, path::Path};

use tempfile::TempDir;
use thinkrail_core::{
    ChangeStatus, DiffRequest, EntryKind, ErrorKind, FileContent, NewWorkspace, Project, WorkspaceId, WorkspaceKind,
    WorkspaceRoots, WorkspaceService,
};

use crate::{GitWorkspaceService, git};

fn sh(root: &Path, args: &[&str]) {
    git::ok(root, args).unwrap();
}

/// A committed repo under a path with CJK characters and spaces, plus its open project.
fn fixture() -> (TempDir, GitWorkspaceService, Project, WorkspaceId) {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("项目 with spaces");
    fs::create_dir(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
    let service = GitWorkspaceService::open(&temp.path().join("data")).unwrap();
    let project = service.open_project(&repo).unwrap();
    let workspace = service.workspaces(&project.id).unwrap()[0].id.clone();
    (temp, service, project, workspace)
}

#[track_caller]
fn kind<T: std::fmt::Debug>(result: thinkrail_core::Result<T>, expected: ErrorKind) {
    assert_eq!(result.unwrap_err().kind(), Some(expected));
}

#[test]
fn project_lifecycle_persists_and_default_is_protected() {
    let (temp, service, project, workspace) = fixture();
    assert_eq!(service.workspace_root(&workspace).unwrap(), fs::canonicalize(&project.path).unwrap());
    kind(service.remove_workspace(&workspace), ErrorKind::Forbidden);
    assert_eq!(service.rename_project(&project.id, " 新的 项目 ").unwrap().name, "新的 项目");
    service.close_project(&project.id).unwrap();
    assert!(service.projects().unwrap().is_empty());
    let reopened = GitWorkspaceService::open(&temp.path().join("data")).unwrap();
    let opened = reopened.open_project(&project.path).unwrap();
    assert_eq!((opened.id, opened.name.as_str()), (project.id.clone(), "新的 项目"));
    let workspaces = reopened.workspaces(&project.id).unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].kind, WorkspaceKind::Default);
    kind(reopened.open_project(&temp.path().join("missing")), ErrorKind::NotFound);
}

#[test]
fn paths_unicode_diff_and_dirty_worktree_protection() {
    let (_temp, service, project, workspace) = fixture();
    let root = service.workspace_root(&workspace).unwrap();
    fs::create_dir(root.join("目录")).unwrap();
    fs::write(root.join("目录/中文 空格.txt"), "你好\n").unwrap();
    for path in ["../secret", "/etc/passwd", "C:\\secret", ".git/config", ".GIT/config", ".git./config", "目录/../../secret", "目录/file:stream"] {
        assert!(service.read_file(&workspace, path).is_err(), "{path}");
    }
    assert_eq!(service.read_file(&workspace, "目录\\中文 空格.txt").unwrap(), FileContent::Text("你好\n".into()));
    let listing = service.read_dir(&workspace, "").unwrap();
    assert_eq!((listing[0].name.as_str(), listing[0].kind), ("目录", EntryKind::Dir));
    assert!(listing.iter().all(|e| e.name != ".git"));
    assert_eq!(service.read_dir(&workspace, "目录").unwrap()[0].path, "目录/中文 空格.txt");
    fs::write(root.join("bin.dat"), [0u8, 159, 146, 150]).unwrap();
    assert_eq!(service.read_file(&workspace, "bin.dat").unwrap(), FileContent::Binary { byte_len: 4 });
    fs::remove_file(root.join("bin.dat")).unwrap();

    let status = service.git_status(&workspace).unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!((status.changes[0].path.as_str(), status.changes[0].status), ("目录/中文 空格.txt", ChangeStatus::Untracked));
    let path = Some("目录/中文 空格.txt".to_owned());
    assert!(service.git_diff(&workspace, &DiffRequest { path: path.clone(), staged: false }).unwrap().contains("+你好"));
    sh(&root, &["add", "--", "目录/中文 空格.txt"]);
    assert!(service.git_diff(&workspace, &DiffRequest { path: None, staged: true }).unwrap().contains("+你好"));
    sh(&root, &["-c", "user.name=测试", "-c", "user.email=t@example.com", "commit", "-q", "-m", "中文提交"]);
    let commits = service.git_commits(&workspace, 10).unwrap();
    assert_eq!((commits[0].subject.as_str(), commits[0].author.as_str()), ("中文提交", "测试"));
    assert!(commits[0].timestamp > 1_600_000_000_000);
    fs::remove_file(root.join("目录/中文 空格.txt")).unwrap();
    assert!(service.git_diff(&workspace, &DiffRequest { path, staged: false }).unwrap().contains("-你好"));
    fs::write(root.join("目录/重命名.txt"), "你好\n").unwrap();
    sh(&root, &["add", "-A"]);
    let renamed = service.git_status(&workspace).unwrap();
    assert_eq!(renamed.changes[0].status, ChangeStatus::Renamed);
    assert_eq!(renamed.changes[0].path, "目录/重命名.txt");
    assert_eq!(renamed.changes[0].original_path.as_deref(), Some("目录/中文 空格.txt"));
    sh(&root, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "rename"]);

    let managed = service.create_workspace(&project.id, NewWorkspace { name: Some("独立 分支".into()), base_ref: None }).unwrap();
    assert_eq!(managed.kind, WorkspaceKind::Managed);
    let path = service.workspace_root(&managed.id).unwrap();
    fs::write(path.join("untracked.txt"), "keep me").unwrap();
    assert!(service.remove_workspace(&managed.id).is_err());
    assert!(path.join("untracked.txt").exists());
    fs::remove_file(path.join("untracked.txt")).unwrap();
    service.remove_workspace(&managed.id).unwrap();
    assert!(!path.exists());
    assert_eq!(service.workspaces(&project.id).unwrap().len(), 1);

    let second = service.create_workspace(&project.id, NewWorkspace::default()).unwrap();
    assert_eq!(second.branch, "workspace-1");
    sh(&root, &["branch", "feature"]);
    service.git_checkout(&second.id, "feature").unwrap();
    assert_eq!(service.git_status(&second.id).unwrap().branch.as_deref(), Some("feature"));
    kind(service.git_checkout(&second.id, "--evil"), ErrorKind::Invalid);
}

#[test]
fn ownership_tampering_never_deletes_foreign_directories() {
    let (temp, service, project, _) = fixture();
    let managed = service.create_workspace(&project.id, NewWorkspace::default()).unwrap();
    let file = temp.path().join("data/workbench.json");
    let original = fs::read_to_string(&file).unwrap();
    let decoy = temp.path().join("decoy");
    fs::create_dir(&decoy).unwrap();
    let decoy_json = serde_json::to_string(&decoy).unwrap();
    let real_json = serde_json::to_string(&service.workspace_root(&managed.id).unwrap()).unwrap();
    let tampered = original.replacen(&real_json[..real_json.len() - 1], &decoy_json[..decoy_json.len() - 1], 1);
    if tampered != original {
        fs::write(&file, &tampered).unwrap();
        assert!(service.remove_workspace(&managed.id).is_err());
        assert!(decoy.exists());
        fs::write(&file, &original).unwrap();
    }
    service.remove_workspace(&managed.id).unwrap();
}

#[test]
fn corrupt_metadata_is_not_silently_replaced() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("workbench.json"), "{broken").unwrap();
    kind(GitWorkspaceService::open(temp.path()), ErrorKind::Corrupt);
    assert_eq!(fs::read_to_string(temp.path().join("workbench.json")).unwrap(), "{broken");
}

#[cfg(windows)]
#[test]
fn junction_cannot_escape_workspace() {
    use std::os::windows::process::CommandExt;
    let (temp, service, _project, workspace) = fixture();
    let root = service.workspace_root(&workspace).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret"), "private").unwrap();
    // cmd is used only to create a junction in this test; production Git never runs a shell.
    let status = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(root.join("escape"))
        .arg(&outside)
        .creation_flags(0x0800_0000)
        .status()
        .unwrap();
    assert!(status.success());
    kind(service.read_file(&workspace, "escape/secret"), ErrorKind::Forbidden);
    assert!(service.read_dir(&workspace, "").unwrap().iter().all(|e| e.name != "escape"));
    fs::remove_dir(root.join("escape")).unwrap();
}

#[test]
fn a_plain_folder_is_initialized_and_never_nested() {
    let temp = TempDir::new().unwrap();
    let folder = temp.path().join("新项目");
    fs::create_dir_all(folder.join("src")).unwrap();
    let service = GitWorkspaceService::open(&temp.path().join("data")).unwrap();
    assert!(!service.is_repository(&folder));
    kind(service.open_project(&folder), ErrorKind::Invalid);

    let project = service.init_project(&folder).unwrap();
    assert!(service.is_repository(&folder));
    let workspaces = service.workspaces(&project.id).unwrap();
    assert_eq!(workspaces[0].branch, "main");
    assert!(git::ok(&folder, &["rev-parse", "--verify", "HEAD"]).is_ok(), "an initial commit exists, so worktrees work");

    let again = service.init_project(&folder.join("src")).unwrap();
    assert_eq!(again.id, project.id, "a subfolder opens the enclosing repository");
    assert!(!folder.join("src").join(".git").exists(), "no nested repository");
}

#[test]
fn create_project_makes_an_empty_committed_repo_and_refuses_a_full_folder() {
    let temp = TempDir::new().unwrap();
    let service = GitWorkspaceService::open(&temp.path().join("data")).unwrap();
    let parent = temp.path().join("thinkrail");
    let project = service.create_project(&parent, "天气 app").unwrap();
    assert_eq!(project.name, "天气 app");
    assert!(git::ok(&parent.join("天气 app"), &["rev-parse", "--verify", "HEAD"]).is_ok(), "HEAD exists");

    fs::create_dir(parent.join("full")).unwrap();
    fs::write(parent.join("full").join("keep.txt"), "mine").unwrap();
    kind(service.create_project(&parent, "full"), ErrorKind::Conflict);
    assert!(!parent.join("full").join(".git").exists(), "a refused folder is untouched");
    kind(service.create_project(&parent, "../escape"), ErrorKind::Invalid);
}
