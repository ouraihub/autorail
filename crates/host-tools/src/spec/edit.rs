//! `updateFrontmatterText`: the lossless, frontmatter-only in-place edit behind `spec_update`.
//!
//! The block is edited line-wise per top-level field: untouched fields, comments, nested/unknown
//! fields and field order stay byte-identical; a touched field's lines are rewritten in the
//! canonical dialect; the body is spliced back untouched, a BOM is put back, and the block keeps the
//! line ending of the frontmatter's own first break.

use serde_yaml_ng::Value;

use super::model::{
    BOM, FmValue, ID, IDENTITY_FIELDS, LIST_FIELDS, field_line, is_spec, parse_frontmatter,
    scalar_text, split_frontmatter,
};

#[derive(Debug, Default)]
pub(super) struct FrontmatterEdit {
    pub set: Vec<(String, String)>,
    pub remove: Vec<String>,
    pub add_list: Vec<(String, Vec<String>)>,
    pub remove_list: Vec<(String, Vec<String>)>,
}

/// A top-level field (`key` set) or a run of standalone lines (column-0 comments, leading blanks).
struct Block {
    key: Option<String>,
    lines: Vec<String>,
}

fn starts_field(line: &str) -> bool {
    !line.is_empty()
        && !line.starts_with([' ', '\t', '#'])
        && !(line.starts_with('-') && line[1..].starts_with([' ', '\t']) || line == "-")
}

fn entry(lines: &[String]) -> Option<(String, Value)> {
    let Ok(Value::Mapping(map)) = serde_yaml_ng::from_str::<Value>(&lines.join("\n")) else {
        return None;
    };
    let mut entries = map.into_iter();
    let (key, value) = entries.next()?;
    if entries.next().is_some() {
        return None;
    }
    Some((scalar_text(&key)?, value))
}

fn current_list(block: Option<&Block>) -> Vec<String> {
    match block.and_then(|b| entry(&b.lines)).map(|(_, value)| value) {
        Some(Value::Sequence(items)) => items.iter().filter_map(scalar_text).collect(),
        Some(value) => scalar_text(&value)
            .filter(|s| !s.is_empty())
            .into_iter()
            .collect(),
        None => Vec::new(),
    }
}

struct Blocks(Vec<Block>);

impl Blocks {
    fn find(&self, key: &str) -> Option<&Block> {
        self.0.iter().find(|b| b.key.as_deref() == Some(key))
    }

    /// Replaces the field's own lines (trailing blank lines kept) or appends it at the end.
    fn put(&mut self, key: &str, value: &FmValue) {
        let line = field_line(key, value);
        match self.0.iter_mut().find(|b| b.key.as_deref() == Some(key)) {
            Some(block) => {
                let kept = block
                    .lines
                    .iter()
                    .rev()
                    .take_while(|l| l.trim().is_empty())
                    .count();
                let tail = block.lines.split_off(block.lines.len() - kept);
                block.lines = std::iter::once(line).chain(tail).collect();
            }
            None => self.0.push(Block {
                key: Some(key.to_owned()),
                lines: vec![line],
            }),
        }
    }

    fn delete(&mut self, key: &str) {
        self.0.retain(|b| b.key.as_deref() != Some(key));
    }
}

pub(super) fn update_frontmatter_text(
    file: &str,
    edit: &FrontmatterEdit,
) -> Result<String, String> {
    let split = split_frontmatter(file).ok_or("File has no frontmatter to update.")?;
    let invalid = || "File frontmatter is not valid YAML.".to_owned();
    if !matches!(
        serde_yaml_ng::from_str::<Value>(&split.fm_text),
        Ok(Value::Mapping(_))
    ) {
        return Err(invalid());
    }

    let mut blocks = Blocks(Vec::new());
    for line in split.fm_text.split('\n') {
        match blocks.0.last_mut() {
            Some(block) if !starts_field(line) && !line.starts_with('#') => {
                block.lines.push(line.to_owned())
            }
            _ => blocks.0.push(Block {
                key: None,
                lines: vec![line.to_owned()],
            }),
        }
    }
    for block in &mut blocks.0 {
        if block.lines.first().is_some_and(|l| starts_field(l)) {
            block.key = entry(&block.lines).map(|(key, _)| key);
        }
    }

    for (key, value) in &edit.set {
        if key == ID {
            return Err("Cannot rename a spec's id via set.".into());
        }
        if LIST_FIELDS.contains(&key.as_str()) {
            return Err(format!(
                "Use addList/removeList to edit the list field \"{key}\"."
            ));
        }
        blocks.put(key, &FmValue::Scalar(value.clone()));
    }
    for key in &edit.remove {
        if IDENTITY_FIELDS.contains(&key.as_str()) {
            return Err(format!("Cannot remove protected field \"{key}\"."));
        }
        blocks.delete(key);
    }
    let entries = |group: &[(String, Vec<String>)], field: &str| -> Vec<String> {
        group
            .iter()
            .filter(|(key, _)| key == field)
            .flat_map(|(_, values)| values.iter().cloned())
            .collect()
    };
    for field in LIST_FIELDS {
        let add = entries(&edit.add_list, field);
        if !add.is_empty() {
            let mut next = current_list(blocks.find(field));
            for value in add {
                if !next.contains(&value) {
                    next.push(value);
                }
            }
            blocks.put(field, &FmValue::List(next));
        }
        let remove = entries(&edit.remove_list, field);
        if !remove.is_empty() {
            let next: Vec<String> = current_list(blocks.find(field))
                .into_iter()
                .filter(|v| !remove.contains(v))
                .collect();
            if next.is_empty() {
                blocks.delete(field)
            } else {
                blocks.put(field, &FmValue::List(next))
            }
        }
    }

    let eol = match file.find('\n') {
        Some(i) if i > 0 && file.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    };
    let mut out = String::with_capacity(file.len() + 64);
    if file.starts_with(BOM) {
        out.push(BOM);
    }
    out.push_str("---");
    out.push_str(eol);
    for line in blocks.0.iter().flat_map(|b| &b.lines) {
        out.push_str(line);
        out.push_str(eol);
    }
    out.push_str("---");
    out.push_str(eol);
    let head = out.len();
    out.push_str(split.body);
    match parse_frontmatter(&out[..head]) {
        Some(fm) if is_spec(&fm) => Ok(out),
        Some(_) => Err("Update would leave the file without a valid id and type.".into()),
        None => Err(invalid()),
    }
}
