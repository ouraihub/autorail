//! Durable revisioned Agent profiles and chat bindings: implements `ProfileService`.

mod state;
mod storage;
mod store;
mod validate;

pub use store::JsonProfileStore;

#[cfg(test)]
mod tests;
