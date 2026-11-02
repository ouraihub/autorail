//! The `spec_*` host tools through the public `HostToolHandler` contract.

use std::{fs, path::Path, path::PathBuf, sync::Arc};

use serde_json::{Value, json};
use tempfile::TempDir;
use thinkrail_core::{
    HostToolHandler, HostToolResult, PlanScope, Result, WorkspaceId, WorkspaceRoots,
};
use thinkrail_host_tools::HostTools;
use thinkrail_plan_review::JsonPlanReviewStore;

struct Root(PathBuf);

impl WorkspaceRoots for Root {
    fn workspace_root(&self, _: &WorkspaceId) -> Result<PathBuf> {
        Ok(self.0.clone())
    }
}

/// `outer/project` is the worktree, so escapes into `outer` are observable.
struct Fixture {
    _dirs: (TempDir, TempDir),
    outer: PathBuf,
    root: PathBuf,
    tools: HostTools,
}

fn fixture() -> Fixture {
    let (data, outer) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let root = outer.path().join("project");
    fs::create_dir_all(&root).unwrap();
    let roots: Arc<dyn WorkspaceRoots> = Arc::new(Root(root.clone()));
    let store = Arc::new(JsonPlanReviewStore::open(data.path(), roots.clone()).unwrap());
    Fixture {
        outer: outer.path().to_path_buf(),
        _dirs: (data, outer),
        root,
        tools: HostTools::new(store, roots),
    }
}

impl Fixture {
    fn call(&self, name: &str, args: Value) -> HostToolResult {
        let scope = PlanScope {
            workspace: "w".into(),
            chat: "c".into(),
        };
        let result = self.tools.call(&scope, name, args).unwrap();
        assert!(
            result.text.len() <= 50 * 1024 && result.text.split('\n').count() <= 2000,
            "{name} output unbounded"
        );
        result
    }

    fn ok(&self, name: &str, args: Value) -> String {
        let result = self.call(name, args);
        assert!(!result.is_error, "{name} failed: {}", result.text);
        result.text
    }

    fn err(&self, name: &str, args: Value) -> String {
        let result = self.call(name, args);
        assert!(
            result.is_error,
            "{name} unexpectedly succeeded: {}",
            result.text
        );
        result.text
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root.join(rel)).unwrap()
    }

    fn create(&self, path: &str, id: &str, extra: Value) -> String {
        let mut args =
            json!({"path": path, "id": id, "type": "module-design", "title": id.to_uppercase()});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().cloned().unwrap_or_default());
        self.ok("spec_create", args)
    }
}

fn spec(id: &str, extra: &str) -> String {
    format!("---\nid: {id}\ntype: module-design\ntitle: {id}\n{extra}---\n")
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

#[test]
fn create_scaffolds_canonical_frontmatter_and_get_reads_it_back() {
    let f = fixture();
    let text = f.create(
        "./deep/nested/../nested/SPEC.md",
        "m",
        json!({"status": "active", "parent": "p", "dependsOn": ["d1", "d2"], "references": ["r1"],
               "implements": ["i1"], "covers": ["c1"], "tags": ["hello, world", "t2"]}),
    );
    assert!(
        text.contains("Created deep/nested/SPEC.md (id: m)."),
        "{text}"
    );
    assert_eq!(
        f.read("deep/nested/SPEC.md"),
        "---\nid: m\ntype: module-design\nstatus: active\ntitle: M\nparent: p\ndepends-on: [d1, d2]\n\
         references: [r1]\nimplements: [i1]\ncovers: [c1]\ntags: [\"hello, world\", t2]\n---\n\n\
         ## Responsibility\n\n## Boundary\n"
    );
    let got = f.ok("spec_get", json!({"id": "m"}));
    assert!(
        got.starts_with("m [module-design] — M\npath: deep/nested/SPEC.md\n"),
        "{got}"
    );
    assert!(got.contains("  parent -> p (missing)"), "{got}");

    f.create("t.md", "t", json!({"type": "task-spec", "title": "T"}));
    assert!(f.read("t.md").ends_with("## Purpose\n\n## Open items\n"));
    // Values YAML would read as another type or structure stay strings.
    f.create("q.md", "007", json!({"title": "yes: no #x"}));
    let got = f.ok("spec_get", json!({"id": "007"}));
    assert!(
        got.starts_with("007 [module-design] — yes: no #x\n"),
        "{got}"
    );
}

#[test]
fn create_refuses_existing_paths_used_ids_and_non_specs() {
    let f = fixture();
    f.create("pkg/SPEC.md", "pkg", json!({}));
    assert!(
        f.err(
            "spec_create",
            json!({"path": "other.md", "id": "pkg", "type": "task-spec", "title": "D"})
        )
        .contains("already in use")
    );
    assert!(
        f.err(
            "spec_create",
            json!({"path": "pkg/SPEC.md", "id": "x", "type": "task-spec", "title": "E"})
        )
        .contains("already exists")
    );
    f.err(
        "spec_create",
        json!({"path": "blank.md", "id": "", "type": "module-design", "title": "T"}),
    );
    assert!(!exists(&f.root.join("blank.md")));
    assert!(!exists(&f.root.join("other.md")));
}

#[test]
fn create_refuses_paths_the_index_could_never_see() {
    let f = fixture();
    fs::create_dir_all(f.root.join("node_modules")).unwrap();
    let absolute = f.outer.join("abs.md").to_string_lossy().into_owned();
    for path in [
        "../escape.md",
        "pkg/../../escape.md",
        absolute.as_str(),
        "/tmp/escape.md",
        "notes/spec.txt",
        "node_modules/dep/SPEC.md",
        "NODE_MODULES/SPEC.md",
        ".git/SPEC.md",
        "Build/x/SPEC.md",
        "",
    ] {
        f.err(
            "spec_create",
            json!({"path": path, "id": "x", "type": "module-design", "title": "T"}),
        );
    }
    if cfg!(windows) {
        for path in ["C:..\\..\\outside.md", "notes.txt:SPEC.md", "C:SPEC.md"] {
            let text = f.err(
                "spec_create",
                json!({"path": path, "id": "x", "type": "module-design", "title": "T"}),
            );
            assert!(text.contains("Windows drive or stream syntax"), "{text}");
        }
        f.err(
            "spec_create",
            json!({"path": "dir./SPEC.md", "id": "x", "type": "module-design", "title": "T"}),
        );
    }
    assert!(!exists(&f.outer.join("escape.md")) && !exists(&f.outer.join("abs.md")));
    assert!(
        !exists(&f.root.join("notes"))
            && !exists(&f.root.join("node_modules/SPEC.md"))
            && !exists(&f.root.join(".git"))
    );
}

#[test]
fn create_writes_nothing_through_a_symlink_or_junction() {
    let f = fixture();
    let elsewhere = f.outer.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let linked = link_dir(&elsewhere, &f.root.join("link"));
    if !linked {
        eprintln!("skipping: cannot create a directory link here");
        return;
    }
    f.err(
        "spec_create",
        json!({"path": "link/evil.md", "id": "e", "type": "module-design", "title": "E"}),
    );
    assert!(!exists(&elsewhere.join("evil.md")));
    // The glob never follows the link either.
    fs::write(elsewhere.join("seen.md"), spec("seen", "")).unwrap();
    f.err("spec_get", json!({"id": "seen"}));
}

#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
        || std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[test]
fn update_rewrites_only_touched_fields_and_keeps_every_other_byte() {
    let f = fixture();
    let body = "\n# Body\nplain line\r\nanother\n- a bullet\n";
    f.write(
        "m.md",
        &format!(
            "---\nid: m # the slug\ntype: module-design\n# a standalone note\ntitle: M\nowner:\n  name: bob # keep me\n  team: infra\ntags: [x, y]\nparent: p\n---{body}"
        ),
    );
    f.ok(
        "spec_update",
        json!({"id": "m", "set": {"title": "M v2", "status": "active"}, "remove": ["parent"],
               "addList": {"tags": ["y", "z"], "depends-on": ["d1", "d1", "d2"], "references": ["r1"]}}),
    );
    f.ok(
        "spec_update",
        json!({"id": "m", "removeList": {"references": ["r1"]}}),
    );
    assert_eq!(
        f.read("m.md"),
        format!(
            "---\nid: m # the slug\ntype: module-design\n# a standalone note\ntitle: M v2\nowner:\n  name: bob # keep me\n  team: infra\ntags: [x, y, z]\nstatus: active\ndepends-on: [d1, d2]\n---{body}"
        )
    );
}

#[test]
fn update_keeps_crlf_bom_and_block_lists_readable() {
    let f = fixture();
    f.write("c.md", "\u{feff}---\r\nid: c\r\ntype: module-design\r\ncovers:\r\n  - a\r\n  - b\r\ntitle: T\r\n---\r\nbody\nline\r\n");
    f.ok(
        "spec_update",
        json!({"id": "c", "addList": {"covers": ["c"]}, "set": {"title": "T2"}}),
    );
    assert_eq!(
        f.read("c.md"),
        "\u{feff}---\r\nid: c\r\ntype: module-design\r\ncovers: [a, b, c]\r\ntitle: T2\r\n---\r\nbody\nline\r\n"
    );
    f.ok(
        "spec_update",
        json!({"id": "c", "removeList": {"covers": ["a", "b", "c"]}}),
    );
    assert!(!f.read("c.md").contains("covers"));
    assert!(
        f.ok("spec_get", json!({"id": "c"}))
            .starts_with("c [module-design] — T2\n")
    );
}

#[test]
fn update_never_unspecs_renames_or_list_coerces() {
    let f = fixture();
    f.create("m/SPEC.md", "m", json!({}));
    let before = f.read("m/SPEC.md");
    for args in [
        json!({"id": "m", "set": {"id": "m2"}}),
        json!({"id": "m", "set": {"type": ""}}),
        json!({"id": "m", "remove": ["id"]}),
        json!({"id": "m", "remove": ["type"]}),
        json!({"id": "m", "set": {"tags": "a, b"}}),
        json!({"id": "ghost", "set": {"title": "x"}}),
    ] {
        f.err("spec_update", args);
    }
    assert_eq!(f.read("m/SPEC.md"), before);
    f.ok(
        "spec_update",
        json!({"id": "m", "set": {"type": "submodule-design"}}),
    );
    assert!(
        f.ok("spec_get", json!({"id": "m"}))
            .starts_with("m [submodule-design]")
    );
}

#[test]
fn grep_pages_by_count_and_bytes_without_losing_or_repeating_matches() {
    let f = fixture();
    let lines: Vec<String> = (0..240)
        .map(|i| format!("needle {i} {}", "x".repeat(1000)))
        .collect();
    f.write(
        "SPEC.md",
        &format!(
            "---\nid: large\ntype: module-design\n---\n{}",
            lines.join("\n")
        ),
    );
    let (mut found, mut offset) = (Vec::new(), 0);
    loop {
        let text = f.ok("spec_grep", json!({"pattern": "needle", "offset": offset}));
        found.extend(text.lines().filter_map(|l| {
            l.strip_prefix("SPEC.md:")
                .map(|m| m.split_once(": ").unwrap().1.to_owned())
        }));
        let Some(next) = text
            .rsplit_once("offset: ")
            .map(|(_, rest)| rest.split(',').next().unwrap().parse::<usize>().unwrap())
        else {
            break;
        };
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(found, lines);

    f.write(
        "many.md",
        &format!(
            "---\nid: many\ntype: task-spec\ntags: [target]\n---\n{}",
            "pin\n".repeat(250)
        ),
    );
    let first = f.ok(
        "spec_grep",
        json!({"pattern": "pin", "tag": "target", "limit": 100000}),
    );
    assert!(
        first.starts_with("200 match(es) (truncated):")
            && first.ends_with("offset: 200, or narrow the search."),
        "{}",
        &first[..60]
    );
    let last = f.ok(
        "spec_grep",
        json!({"pattern": "pin", "tag": "target", "offset": 200}),
    );
    assert!(last.starts_with("50 match(es):") && !last.contains("More matches"));
    assert_eq!(
        f.err("spec_grep", json!({"pattern": "pin", "offset": -1})),
        "Error: Use a nonnegative integer offset."
    );
}

#[test]
fn grep_matches_crlf_bom_regex_case_and_metadata_filters() {
    let f = fixture();
    f.write("a.md", "\u{feff}---\r\nid: a\r\ntype: module-design\r\ntags: [x]\r\nparent: p\r\ndepends-on: [b]\r\n---\r\nHello world\r\n");
    f.write("b.md", "---\nid: b\ntype: task-spec\n---\nhello there\n");
    f.write("notes.md", "hello but not a spec\n");
    let paths = |args: Value| -> Vec<String> {
        f.ok("spec_grep", args)
            .lines()
            .skip(1)
            .map(|l| l.split(':').next().unwrap().to_owned())
            .collect()
    };
    assert_eq!(paths(json!({"pattern": "hello"})), ["a.md", "b.md"]);
    assert_eq!(
        paths(json!({"pattern": "hello", "ignoreCase": false})),
        ["b.md"]
    );
    assert_eq!(
        paths(json!({"pattern": "^hello.*d$", "regex": true})),
        ["a.md"]
    );
    assert_eq!(
        paths(json!({"pattern": "^---$", "regex": true, "type": "module-design"})),
        ["a.md", "a.md"]
    );
    for filter in [
        json!({"tag": "x"}),
        json!({"parent": "p"}),
        json!({"dependsOn": "b"}),
    ] {
        let mut args = json!({"pattern": "hello"});
        args.as_object_mut()
            .unwrap()
            .extend(filter.as_object().unwrap().clone());
        assert_eq!(paths(args), ["a.md"]);
    }
    assert_eq!(f.ok("spec_grep", json!({"pattern": "zzz"})), "No matches.");
    assert!(
        f.err("spec_grep", json!({"pattern": "(", "regex": true}))
            .starts_with("Error: Invalid search pattern")
    );
}

fn seed_tree(f: &Fixture) {
    f.write("root.md", &spec("root", ""));
    f.write("a/SPEC.md", &spec("a", "parent: root\ndepends-on: [b]\n"));
    f.write(
        "b/SPEC.md",
        &spec("b", "parent: root\ndepends-on: [c, ghost]\n"),
    );
    f.write(
        "a/sub/SPEC.md",
        &spec("sub", "parent: a\nimplements: [a]\n"),
    );
    f.write("c.md", &spec("c", ""));
}

fn slice_nodes(text: &str) -> Vec<&str> {
    let nodes = text
        .split("nodes (")
        .nth(1)
        .unwrap()
        .split("edges (")
        .next()
        .unwrap();
    let mut ids: Vec<&str> = nodes
        .lines()
        .skip(1)
        .map(|l| l.trim().split(' ').next().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

#[test]
fn graph_slices_walk_each_direction_bounded_by_depth() {
    let f = fixture();
    seed_tree(&f);
    let slice = |args: Value| f.ok("spec_graph", args);
    assert_eq!(
        slice_nodes(&slice(json!({"root": "root", "direction": "subtree"}))),
        ["a", "b", "root"]
    );
    assert_eq!(
        slice_nodes(&slice(
            json!({"root": "root", "direction": "subtree", "depth": 2})
        )),
        ["a", "b", "root", "sub"]
    );
    assert_eq!(
        slice_nodes(&slice(
            json!({"root": "sub", "direction": "ancestors", "depth": 5})
        )),
        ["a", "root", "sub"]
    );
    assert_eq!(
        slice_nodes(&slice(json!({"root": "b", "direction": "neighbors"}))),
        ["a", "b", "c"]
    );
    assert_eq!(
        slice_nodes(&slice(
            json!({"root": "a", "direction": "neighbors", "edge": "implements"})
        )),
        ["a", "sub"]
    );
    let two = slice(json!({"root": "a", "direction": "neighbors", "depth": 2}));
    assert!(
        two.contains("edges (3):") && two.contains("missing targets: ghost"),
        "{two}"
    );
    f.err(
        "spec_graph",
        json!({"root": "nope", "direction": "subtree"}),
    );
}

#[test]
fn get_resolves_forward_and_reverse_links() {
    let f = fixture();
    seed_tree(&f);
    let got = f.ok("spec_get", json!({"id": "a"}));
    assert!(
        got.contains("links:\n  parent -> root (root.md)\n  depends-on -> b (b/SPEC.md)\n"),
        "{got}"
    );
    assert!(
        got.contains(
            "referenced by:\n  parent -> sub (a/sub/SPEC.md)\n  implements -> sub (a/sub/SPEC.md)"
        ),
        "{got}"
    );
    assert!(
        f.ok("spec_get", json!({"id": "b"}))
            .contains("depends-on -> ghost (missing)")
    );
    assert_eq!(
        f.err("spec_get", json!({"id": "nope"})),
        "Error: No spec with id \"nope\"."
    );
}

#[test]
fn validate_reports_dangling_links_duplicates_and_cycles_and_ignores_scratch_dirs() {
    let f = fixture();
    f.write("a.md", &spec("a", ""));
    assert_eq!(
        f.ok("spec_validate", json!({})),
        "Spec-graph is valid: no issues found."
    );
    f.ok("spec_update", json!({"id": "a", "set": {"parent": "gp"}, "addList": {"depends-on": ["gd"], "references": ["gr"], "implements": ["gi"]}}));
    f.write("b1.md", &spec("b", ""));
    f.write("b2.md", &spec("b", ""));
    f.write("c1.md", &spec("c1", "parent: c2\n"));
    f.write("c2.md", &spec("c2", "parent: c1\n"));
    for dir in [
        "node_modules/dep",
        ".git",
        ".claude/x",
        "dist",
        "build",
        ".superpowers",
        ".gsd",
        ".mockups",
        ".playwright-mcp",
    ] {
        f.write(&format!("{dir}/SPEC.md"), &spec("b", "parent: nowhere\n"));
    }
    let report = f.ok("spec_validate", json!({}));
    assert_eq!(
        report,
        "Duplicate ids (1):\n  b: b1.md, b2.md\n\n\
         Dangling links (4):\n  a (a.md) --parent--> gp [missing]\n  a (a.md) --depends-on--> gd [missing]\n  \
         a (a.md) --references--> gr [missing]\n  a (a.md) --implements--> gi [missing]\n\n\
         Parent cycles (1):\n  c1 -> c2 -> c1"
    );
}

#[test]
fn delete_removes_the_winning_file_and_leaves_links_dangling() {
    let f = fixture();
    seed_tree(&f);
    f.err("spec_delete", json!({"id": "ghost"}));
    assert_eq!(
        f.ok("spec_delete", json!({"id": "b"})),
        "Deleted b/SPEC.md (id: b)."
    );
    assert!(!exists(&f.root.join("b/SPEC.md")));
    assert!(
        f.ok("spec_validate", json!({}))
            .contains("a (a/SPEC.md) --depends-on--> b [missing]")
    );
}

#[test]
fn oversized_output_is_capped_and_saved_to_a_private_temp_file() {
    let f = fixture();
    let refs: Vec<String> = (0..2200).map(|i| format!("missing-{i}")).collect();
    f.write(
        "SPEC.md",
        &spec("root", &format!("references: [{}]\n", refs.join(", "))),
    );
    for (name, args) in [
        ("spec_get", json!({"id": "root"})),
        ("spec_validate", json!({})),
        (
            "spec_graph",
            json!({"root": "root", "direction": "neighbors", "edge": "references"}),
        ),
    ] {
        let text = f.ok(name, args);
        let path = text
            .split("Full output saved to: ")
            .nth(1)
            .and_then(|rest| rest.lines().next())
            .expect("capture path");
        assert!(fs::read_to_string(path).unwrap().contains("missing-2199"));
        fs::remove_file(path).unwrap();
    }
    let huge = "x".repeat(60_000);
    let text = f.err("spec_get", json!({"id": huge}));
    assert!(
        text.starts_with("Error:\n") && text.contains("[Output truncated:"),
        "{}",
        &text[..40]
    );
    if let Some(path) = text
        .split("Full output saved to: ")
        .nth(1)
        .and_then(|rest| rest.lines().next())
    {
        fs::remove_file(path).unwrap();
    }
}
