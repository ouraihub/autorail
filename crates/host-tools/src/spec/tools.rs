//! One function per `spec_*` tool (original `tools/*.ts`): parse params, run the model, format text.
//! `Ok((text, continuation))` is a result; `Err(message)` a tool-level failure.

use std::{fs, io::Write, path::Path};

use regex::RegexBuilder;
use serde_json::Value;

use super::GREP_PAGE_BYTES;
use super::edit::{FrontmatterEdit, update_frontmatter_text};
use super::graph::{Graph, dangling_links, kind_index, link_targets, parent_cycles, slice};
use super::model::{
    COVERS, DEPENDS_ON, FmValue, ID, IMPLEMENTS, LINK_KINDS, PARENT, REFERENCES, SLICE_DIRECTIONS, STATUS, TAGS, TITLE,
    TYPE, is_spec, list, parse_frontmatter, scalar, serialize_frontmatter,
};
use super::store::{resolve_spec_path, scan};

pub(super) const NAMES: [&str; 7] = ["spec_grep", "spec_get", "spec_graph", "spec_create", "spec_update", "spec_delete", "spec_validate"];
const SPEC_TYPES: [&str; 5] = ["goal-and-requirements", "architecture-design", "module-design", "submodule-design", "task-spec"];
const SPEC_STATUSES: [&str; 5] = ["draft", "active", "stale", "done", "deprecated"];
const DEFAULT_GREP_LIMIT: usize = 200;

type Outcome = Result<(String, String), String>;

pub(super) fn run(root: &Path, name: &str, args: &Value) -> Outcome {
    match name {
        "spec_grep" => grep(root, args),
        "spec_get" => get(root, args),
        "spec_graph" => graph(root, args),
        "spec_create" => create(root, args),
        "spec_update" => update(root, args),
        "spec_delete" => delete(root, args),
        "spec_validate" => validate(root),
        other => Err(format!("Unknown host tool \"{other}\".")),
    }
}

fn done(text: impl Into<String>) -> Outcome {
    Ok((text.into(), String::new()))
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| format!("`{key}` is required."))
}

fn optional<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` must be a string.")),
    }
}

fn strings(value: Option<&Value>, key: &str) -> Result<Vec<String>, String> {
    let bad = || format!("`{key}` must be an array of strings.");
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items.iter().map(|v| v.as_str().map(str::to_owned).ok_or_else(bad)).collect(),
        Some(_) => Err(bad()),
    }
}

fn one_of<'a>(value: Option<&'a str>, key: &str, allowed: &[&str]) -> Result<Option<&'a str>, String> {
    match value {
        Some(v) if !allowed.contains(&v) => Err(format!("Unknown {key} \"{v}\"; use {}.", allowed.join(" | "))),
        other => Ok(other),
    }
}

fn no_spec(id: &str) -> String {
    format!("No spec with id \"{id}\".")
}

fn grep(root: &Path, args: &Value) -> Outcome {
    let offset = match args.get("offset") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("Use a nonnegative integer offset.")?,
    };
    let pattern = required(args, "pattern")?;
    let requested = args.get("limit").and_then(Value::as_f64).map_or(0.0, f64::trunc);
    let limit = if requested >= 1.0 { (requested.min(DEFAULT_GREP_LIMIT as f64)) as usize } else { DEFAULT_GREP_LIMIT };
    let ignore_case = args.get("ignoreCase").and_then(Value::as_bool).unwrap_or(true);
    let matcher: Box<dyn Fn(&str) -> bool> = if args.get("regex").and_then(Value::as_bool).unwrap_or(false) {
        let re = RegexBuilder::new(pattern).case_insensitive(ignore_case).build().map_err(|e| format!("Invalid search pattern: {e}"))?;
        Box::new(move |line| re.is_match(line))
    } else if ignore_case {
        let needle = pattern.to_lowercase();
        Box::new(move |line| line.to_lowercase().contains(&needle))
    } else {
        Box::new(move |line| line.contains(pattern))
    };
    let (kind, tag, parent, depends) =
        (optional(args, "type")?, optional(args, "tag")?, optional(args, "parent")?, optional(args, "dependsOn")?);

    let mut matches: Vec<String> = Vec::new();
    let (mut skipped, mut bytes, mut next_offset) = (0, 0, None);
    'files: for file in &scan(root) {
        let fm = &file.fm;
        if kind.is_some_and(|k| scalar(fm, TYPE) != Some(k))
            || parent.is_some_and(|p| scalar(fm, PARENT) != Some(p))
            || tag.is_some_and(|t| !list(fm, TAGS).contains(&t))
            || depends.is_some_and(|d| !list(fm, DEPENDS_ON).contains(&d))
        {
            continue;
        }
        let content = file.content.strip_prefix('\u{feff}').unwrap_or(&file.content);
        for (i, raw) in content.split('\n').enumerate() {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if !matcher(line) {
                continue;
            }
            if skipped < offset {
                skipped += 1;
                continue;
            }
            let formatted = format!("{}:{}: {}", file.rel, i + 1, line.trim());
            if matches.len() >= limit || (!matches.is_empty() && bytes + formatted.len() + 1 > GREP_PAGE_BYTES) {
                next_offset = Some(offset + matches.len());
                break 'files;
            }
            bytes += formatted.len() + 1;
            matches.push(formatted);
        }
    }
    let header = match (matches.len(), next_offset) {
        (0, _) => "No matches.".to_owned(),
        (n, Some(_)) => format!("{n} match(es) (truncated):"),
        (n, None) => format!("{n} match(es):"),
    };
    let text = format!("{header}\n{}", matches.join("\n")).trim_end().to_owned();
    let continuation = next_offset.map_or_else(String::new, |next| {
        format!("More matches: repeat spec_grep with the same filters and offset: {next}, or narrow the search.")
    });
    Ok((text, continuation))
}

fn get(root: &Path, args: &Value) -> Outcome {
    let id = required(args, "id")?;
    let files = scan(root);
    let graph = Graph::build(&files);
    let node = graph.node(id).ok_or_else(|| no_spec(id))?;
    let fmt = |kind: &str, target: &str| match graph.node(target) {
        Some(n) => format!("  {kind} -> {target} ({})", n.file.rel),
        None => format!("  {kind} -> {target} (missing)"),
    };
    let mut links = Vec::new();
    let mut reverse = Vec::new();
    for (k, kind) in LINK_KINDS.iter().enumerate() {
        links.extend(link_targets(node.file, kind).into_iter().map(|t| fmt(kind, t)));
        reverse.extend(graph.reverse(k, id).iter().map(|s| fmt(kind, s)));
    }
    let title = node.title.map_or_else(String::new, |t| format!(" — {t}"));
    let section = |label: &str, lines: Vec<String>| {
        if lines.is_empty() { format!("{label}: (none)") } else { format!("{label}:\n{}", lines.join("\n")) }
    };
    done(format!(
        "{} [{}]{title}\npath: {}\n{}\n{}",
        node.id,
        node.kind,
        node.file.rel,
        section("links", links),
        section("referenced by", reverse)
    ))
}

fn graph(root: &Path, args: &Value) -> Outcome {
    let start = required(args, "root")?;
    let direction = one_of(Some(required(args, "direction")?), "direction", &SLICE_DIRECTIONS)?.unwrap_or("subtree");
    let edge = one_of(optional(args, "edge")?, "edge", &LINK_KINDS)?.and_then(kind_index).unwrap_or(1);
    let depth = match args.get("depth") {
        None | Some(Value::Null) => 1.0,
        Some(v) => v.as_f64().ok_or("`depth` must be a number.")?,
    };
    let files = scan(root);
    let graph = Graph::build(&files);
    let Some(node) = graph.node(start) else { return Err(no_spec(start)) };
    let slice = slice(&graph, node.id, direction, edge, depth);
    let mut lines = vec![format!("Slice of \"{start}\" ({direction}, depth {depth}):"), format!("nodes ({}):", slice.nodes.len())];
    for n in &slice.nodes {
        let title = n.title.map_or_else(String::new, |t| format!(" — {t}"));
        lines.push(format!("  {} [{}]{title} ({})", n.id, n.kind, n.file.rel));
    }
    lines.push(format!("edges ({}):", slice.edges.len()));
    lines.extend(slice.edges.iter().map(|e| format!("  {} --{}--> {}", e.from, e.kind, e.to)));
    if !slice.missing.is_empty() {
        lines.push(format!("missing targets: {}", slice.missing.join(", ")));
    }
    done(lines.join("\n"))
}

fn scaffold(kind: &str) -> String {
    let headings: &[&str] = match kind {
        "module-design" | "submodule-design" => &["Responsibility", "Boundary"],
        "architecture-design" => &["Drivers", "Decisions", "Invariants", "Out of scope"],
        "goal-and-requirements" => &["Goal", "Scope"],
        "task-spec" => &["Purpose", "Open items"],
        _ => &[],
    };
    headings.iter().map(|h| format!("## {h}\n")).collect::<Vec<_>>().join("\n")
}

fn create(root: &Path, args: &Value) -> Outcome {
    let (path, id, title) = (required(args, "path")?, required(args, "id")?, required(args, "title")?);
    let kind = one_of(Some(required(args, "type")?), "type", &SPEC_TYPES)?.unwrap_or_default();
    let status = one_of(optional(args, "status")?, "status", &SPEC_STATUSES)?;
    let (rel, abs) = resolve_spec_path(root, path)?;
    if fs::symlink_metadata(&abs).is_ok() {
        return Err(format!("File already exists: {rel}"));
    }
    if Graph::build(&scan(root)).node(id).is_some() {
        return Err(format!("Spec id \"{id}\" is already in use."));
    }
    let scalar = |s: Option<&str>| FmValue::Scalar(s.unwrap_or_default().to_owned());
    let mut fields = vec![
        (ID, scalar(Some(id))),
        (TYPE, scalar(Some(kind))),
        (STATUS, scalar(status)),
        (TITLE, scalar(Some(title))),
        (PARENT, scalar(optional(args, "parent")?)),
    ];
    for (field, key) in [(DEPENDS_ON, "dependsOn"), (REFERENCES, "references"), (IMPLEMENTS, "implements"), (COVERS, "covers"), (TAGS, "tags")] {
        fields.push((field, FmValue::List(strings(args.get(key), key)?)));
    }
    let content = format!("{}\n{}", serialize_frontmatter(&fields), scaffold(kind));
    if !parse_frontmatter(&content).is_some_and(|fm| is_spec(&fm)) {
        return Err(format!("Refusing to write {rel}: the frontmatter would not be a spec (id and type must be non-empty)."));
    }
    let written = abs.parent().map_or(Ok(()), fs::create_dir_all).and_then(|()| {
        fs::OpenOptions::new().write(true).create_new(true).open(&abs)?.write_all(content.as_bytes())
    });
    written.map_err(|e| format!("Failed to write {rel}: {e}"))?;
    done(format!("Created {rel} (id: {id})."))
}

fn list_group(args: &Value, key: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => map.iter().map(|(field, v)| Ok((field.clone(), strings(Some(v), field)?))).collect(),
        Some(_) => Err(format!("`{key}` must be an object of string arrays.")),
    }
}

fn update(root: &Path, args: &Value) -> Outcome {
    let id = required(args, "id")?;
    let set = match args.get("set") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())).ok_or_else(|| format!("`set.{k}` must be a string.")))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("`set` must be an object of strings.".into()),
    };
    let edit = FrontmatterEdit { set, remove: strings(args.get("remove"), "remove")?, add_list: list_group(args, "addList")?, remove_list: list_group(args, "removeList")? };
    let files = scan(root);
    let file = files.iter().find(|f| scalar(&f.fm, ID) == Some(id)).ok_or_else(|| no_spec(id))?;
    let content = update_frontmatter_text(&file.content, &edit)?;
    fs::write(&file.abs, content).map_err(|e| format!("Failed to write {}: {e}", file.rel))?;
    done(format!("Updated frontmatter of {} (id: {id}).", file.rel))
}

fn delete(root: &Path, args: &Value) -> Outcome {
    let id = required(args, "id")?;
    let files = scan(root);
    let graph = Graph::build(&files);
    let file = graph.node(id).ok_or_else(|| no_spec(id))?.file;
    fs::remove_file(&file.abs).map_err(|e| format!("Failed to delete {}: {e}", file.rel))?;
    done(format!("Deleted {} (id: {id}).", file.rel))
}

fn validate(root: &Path) -> Outcome {
    let files = scan(root);
    let graph = Graph::build(&files);
    let (dangling, cycles) = (dangling_links(&graph), parent_cycles(&graph));
    let mut sections = Vec::new();
    if !graph.duplicates.is_empty() {
        let lines: Vec<String> = graph.duplicates.iter().map(|(id, paths)| format!("  {id}: {}", paths.join(", "))).collect();
        sections.push(format!("Duplicate ids ({}):\n{}", lines.len(), lines.join("\n")));
    }
    if !dangling.is_empty() {
        let lines: Vec<String> =
            dangling.iter().map(|d| format!("  {} ({}) --{}--> {} [missing]", d.from, d.from_path, d.kind, d.target)).collect();
        sections.push(format!("Dangling links ({}):\n{}", lines.len(), lines.join("\n")));
    }
    if !cycles.is_empty() {
        let lines: Vec<String> = cycles.iter().map(|c| format!("  {} -> {}", c.join(" -> "), c[0])).collect();
        sections.push(format!("Parent cycles ({}):\n{}", lines.len(), lines.join("\n")));
    }
    if sections.is_empty() { done("Spec-graph is valid: no issues found.") } else { done(sections.join("\n\n")) }
}
