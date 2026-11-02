//! One user action = one `op{id=… at=…}` span (native-multi-agent 06 §2). Every line logged while
//! it is entered, on any thread that enters it, starts with that prefix, so `op{id=42` selects
//! the whole chain.

use std::{
    panic::Location,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use tracing::Span;

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A new operation span; `at` is where the action was issued (`#[track_caller]` of the caller).
/// The path is shortened to start at `src/` of its crate.
pub fn op(at: &'static Location<'static>) -> Span {
    let file = at.file().replace('\\', "/");
    let short = file
        .split_once("/src/")
        .map_or(file.as_str(), |(_, rest)| rest);
    tracing::info_span!("op", id = NEXT.fetch_add(1, Ordering::Relaxed), at = %format_args!("{short}:{}", at.line()))
}

/// The closing line of an operation, inside its span.
pub fn finish(started: Instant, ok: bool) {
    let ms = started.elapsed().as_millis();
    if ok {
        tracing::debug!(ms, "op done");
    } else {
        tracing::info!(ms, ok, "op done");
    }
}
