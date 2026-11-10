//! DSH skill commands: ACP lists none, so the host scans the skill dirs DSH loads and offers each
//! user-invocable skill as `/<name>` (DSH's `tool-skill` expands it inside the prompt).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde_yaml_ng::Value as Yaml;
use thinkrail_core::{CommandScope, CommandSource, SlashCommand};
use tracing::{debug, warn};

/// Skill commands for a chat in `cwd`, roots in DSH's priority order: the project's
/// `.dsh/skills` and `.agents/skills`, the host-staged bundled skills, `<DSH home>/skills`,
/// then `~/.agents/skills`.
pub(super) fn commands(cwd: &Path, staged: &Path, dsh_home: &Path) -> Vec<SlashCommand> {
    let mut roots = Vec::new();
    if let Some(project) = cwd.ancestors().find(|dir| dir.join(".git").exists()) {
        roots.push((project.join(".dsh").join("skills"), CommandScope::Project));
        roots.push((
            project.join(".agents").join("skills"),
            CommandScope::Project,
        ));
    }
    roots.push((staged.to_path_buf(), CommandScope::Temporary));
    roots.push((dsh_home.join("skills"), CommandScope::User));
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        roots.push((
            PathBuf::from(home).join(".agents").join("skills"),
            CommandScope::User,
        ));
    }
    scan(&roots)
}

/// First root wins on a duplicate name; the result is sorted by name.
fn scan(roots: &[(PathBuf, CommandScope)]) -> Vec<SlashCommand> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (root, scope) in roots {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                warn!(error_kind = ?error.kind(), "agent.skills.scan_unavailable");
                continue;
            }
        };
        let mut unreadable = 0usize;
        let mut files: Vec<PathBuf> = entries
            .filter_map(|entry| match entry {
                Ok(entry) => skill_file(&entry.path()),
                Err(_) => {
                    unreadable += 1;
                    None
                }
            })
            .collect();
        let mut unreadable_files = 0usize;
        let mut skipped = 0usize;
        files.sort();
        for file in files {
            let Some((name, description)) = read_skill(&file, &mut unreadable_files) else {
                skipped += 1;
                continue;
            };
            if seen.insert(name.clone()) {
                out.push(SlashCommand {
                    name,
                    description: Some(description),
                    hint: None,
                    source: CommandSource::Skill,
                    scope: Some(*scope),
                });
            }
        }
        if unreadable != 0 || unreadable_files != 0 {
            warn!(
                failed_entries = unreadable,
                failed_files = unreadable_files,
                "agent.skills.scan_incomplete"
            );
        }
        debug!(
            command_count = out.len(),
            skipped_count = skipped,
            "agent.skills.scanned"
        );
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `<name>/SKILL.md` or `<name>.md` directly under a root.
fn skill_file(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        let file = path.join("SKILL.md");
        return file.is_file().then_some(file);
    }
    (path.is_file() && path.extension().is_some_and(|e| e == "md")).then(|| path.to_path_buf())
}

/// `(name, description)` of a user-invocable skill with valid frontmatter.
fn read_skill(file: &Path, unreadable: &mut usize) -> Option<(String, String)> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(_) => {
            *unreadable += 1;
            return None;
        }
    };
    let front = frontmatter(&text)?;
    let yaml: Yaml = serde_yaml_ng::from_str(front).ok()?;
    if yaml.get("user-invocable").and_then(Yaml::as_bool) == Some(false) {
        return None;
    }
    let name = yaml.get("name").and_then(Yaml::as_str)?.trim();
    let description = yaml.get("description").and_then(Yaml::as_str)?.trim();
    if !valid_name(name) || description.is_empty() {
        return None;
    }
    Some((name.to_owned(), description.to_owned()))
}

/// The YAML between a leading `---` line and the next `---` line.
fn frontmatter(text: &str) -> Option<&str> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some(&rest[..offset]);
        }
        offset += line.len();
    }
    None
}

/// `^[a-z0-9]+(-[a-z0-9]+)*$`.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests;
