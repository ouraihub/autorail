use std::fs;

use super::*;

fn skill(path: &Path, front: &str) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(path, format!("---\n{front}\n---\n\nbody\n")).unwrap();
}

#[test]
fn roots_merge_in_priority_order_and_sort_by_name() {
    let tmp = tempfile::tempdir().unwrap();
    let (project, user) = (tmp.path().join("p"), tmp.path().join("u"));
    skill(
        &project.join("review/SKILL.md"),
        "name: review\ndescription: project review",
    );
    skill(
        &project.join("alpha.md"),
        "name: alpha\ndescription: flat layout",
    );
    skill(
        &user.join("review/SKILL.md"),
        "name: review\ndescription: user review",
    );
    skill(&user.join("zeta/SKILL.md"), "name: zeta\ndescription: z");
    fs::write(
        user.join("notes.txt"),
        "---\nname: notes\ndescription: x\n---\n",
    )
    .unwrap();
    let commands = scan(&[
        (project, CommandScope::Project),
        (tmp.path().join("missing"), CommandScope::Temporary),
        (user, CommandScope::User),
    ]);
    let rows: Vec<_> = commands
        .iter()
        .map(|c| (c.name.as_str(), c.description.as_deref(), c.scope))
        .collect();
    assert_eq!(
        rows,
        [
            ("alpha", Some("flat layout"), Some(CommandScope::Project)),
            (
                "review",
                Some("project review"),
                Some(CommandScope::Project)
            ),
            ("zeta", Some("z"), Some(CommandScope::User)),
        ]
    );
    assert!(commands.iter().all(|c| c.source == CommandSource::Skill));
}

#[test]
fn invalid_or_hidden_skills_are_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    skill(
        &root.join("hidden/SKILL.md"),
        "name: hidden\ndescription: d\nuser-invocable: false",
    );
    skill(&root.join("bad/SKILL.md"), "name: Bad_Name\ndescription: d");
    skill(&root.join("dash/SKILL.md"), "name: a--b\ndescription: d");
    skill(&root.join("nodesc/SKILL.md"), "name: nodesc");
    skill(&root.join("yaml/SKILL.md"), "name: [unclosed");
    fs::write(root.join("plain.md"), "# no frontmatter\n").unwrap();
    skill(
        &root.join("ok/SKILL.md"),
        "name: ok-2\ndescription: fine\nuser-invocable: true",
    );
    let commands = scan(&[(root.to_path_buf(), CommandScope::Temporary)]);
    let names: Vec<_> = commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["ok-2"]);
}

#[test]
fn names_follow_the_dsh_pattern() {
    for good in ["a", "a1", "my-skill-2"] {
        assert!(valid_name(good), "{good}");
    }
    for bad in ["", "-a", "a-", "a--b", "A", "a_b", "技能"] {
        assert!(!valid_name(bad), "{bad}");
    }
}

#[test]
fn frontmatter_accepts_crlf_and_bom() {
    assert_eq!(
        frontmatter("\u{feff}---\r\nname: x\r\n---\r\nbody"),
        Some("name: x\r\n")
    );
    assert_eq!(frontmatter("name: x\n---\n"), None);
    assert_eq!(frontmatter("---\nname: x\n"), None);
}

#[test]
fn project_roots_come_from_the_nearest_git_ancestor() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("repo");
    fs::create_dir_all(project.join(".git")).unwrap();
    let cwd = project.join("src/deep");
    fs::create_dir_all(&cwd).unwrap();
    skill(
        &project.join(".agents/skills/shared/SKILL.md"),
        "name: shared\ndescription: agents dir",
    );
    skill(
        &tmp.path().join("staged/shared/SKILL.md"),
        "name: shared\ndescription: staged",
    );
    let commands = commands(&cwd, &tmp.path().join("staged"), &tmp.path().join("home"));
    let shared = commands.iter().find(|c| c.name == "shared").unwrap();
    assert_eq!(shared.scope, Some(CommandScope::Project));
    assert_eq!(shared.description.as_deref(), Some("agents dir"));
}
