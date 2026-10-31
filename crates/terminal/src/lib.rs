//! PTY lifecycle, tab catalog and replay: implements `TerminalService`.

mod catalog;
mod decode;
mod runtime;
mod service;
mod shell;
#[cfg(test)]
mod tests;

pub use service::PtyTerminalService;
