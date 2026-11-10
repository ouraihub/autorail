//! The log list's narrowing: 只看问题, the event and layer menus and the free-text filter, applied
//! in that order. Pure functions over [`LogEntry`], so the page only stores the choices.

use thinkrail_observability::LogEntry;

/// Lifecycle events an operation can write, in the order the menu lists them.
pub(super) const EVENTS: [&str; 6] = ["ENTRY", "EXIT", "ERROR", "CANCEL", "TIMEOUT", "UNFINISHED"];

/// What the list is narrowed to; `None` means "all".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Narrow {
    pub only_problems: bool,
    pub event: Option<&'static str>,
    pub layer: Option<String>,
}

impl Narrow {
    /// True when `entry` passes every choice and contains `query` (empty matches all).
    pub(super) fn keeps(&self, entry: &LogEntry, query: &str) -> bool {
        (!self.only_problems || entry.level == "ERROR" || entry.level == "WARN")
            && self
                .event
                .is_none_or(|event| entry.event.as_deref() == Some(event))
            && self
                .layer
                .as_deref()
                .is_none_or(|layer| entry.layer.as_deref() == Some(layer))
            && (query.is_empty() || entry.raw.contains(query))
    }
}

/// The layers present in `entries`, sorted, for the layer menu.
pub(super) fn layers(entries: &[LogEntry]) -> Vec<String> {
    let mut layers: Vec<String> = entries.iter().filter_map(|e| e.layer.clone()).collect();
    layers.sort_unstable();
    layers.dedup();
    layers
}

#[cfg(test)]
mod tests;
