//! Request/reply correlation and reader-thread callback execution for a live session.

use std::{
    sync::{atomic::Ordering, mpsc},
    time::Duration,
};

use serde_json::Value;
use thinkrail_core::{Error, ErrorCode as _, Result};
use thinkrail_observability::OperationGuard;
use tracing::debug;

use super::{Live, closed};

type Callback = Box<dyn FnOnce(Result<Value>) + Send>;

pub(super) struct Waiter {
    callback: Callback,
    trace: std::sync::Arc<crate::trace::SessionTrace>,
    operation: OperationGuard,
}

impl Waiter {
    pub(super) fn complete(self, result: Result<Value>) {
        let _entered = self.trace.enter_operation(&self.operation);
        let is_error = result.is_err();
        if let Err(error) = &result {
            self.operation.fail(error.code());
        } else {
            self.operation.finish("SUCCESS");
        }
        tracing::info!(is_error, "agent.request.replied");
        (self.callback)(result);
    }

    pub(super) fn close(self) {
        let _entered = self.trace.enter_operation(&self.operation);
        self.operation.fail("SESSION_CLOSED");
        tracing::info!("agent.request.closed");
        (self.callback)(Err(closed()));
    }

    pub(super) fn cancel(self, reason: &'static str) {
        let _entered = self.trace.enter_operation(&self.operation);
        self.operation.cancel(reason);
    }

    pub(super) fn timeout(self, limit_ms: u64) {
        let _entered = self.trace.enter_operation(&self.operation);
        self.operation.timeout(limit_ms);
    }
}

impl Live {
    /// Registers `waiter` for the reply keyed `key`. Waiters run on the reader thread, in frame
    /// order, before the next frame is handled.
    pub(crate) fn on_reply(&self, key: impl Into<String>, callback: Callback) -> Result<()> {
        let mut pending = self.pending.lock();
        if self.closed.load(Ordering::Acquire) {
            return Err(closed());
        }
        let key = key.into();
        let operation = self.process.trace.operation("agent.request", "external");
        pending.insert(
            key,
            Waiter {
                callback,
                trace: self.process.trace.clone(),
                operation,
            },
        );
        Ok(())
    }

    pub(crate) fn expect(&self, key: impl Into<String>) -> Result<mpsc::Receiver<Result<Value>>> {
        self.expect_then(key, |_| {})
    }

    /// Like [`Self::expect`], running `hook` on the reader thread when a successful reply arrives.
    pub(crate) fn expect_then(
        &self,
        key: impl Into<String>,
        hook: impl FnOnce(&Value) + Send + 'static,
    ) -> Result<mpsc::Receiver<Result<Value>>> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.on_reply(
            key,
            Box::new(move |result| {
                if let Ok(value) = &result {
                    hook(value);
                }
                if sender.send(result).is_err() {
                    debug!("agent reply arrived after its waiter gave up");
                }
            }),
        )?;
        Ok(receiver)
    }

    pub(crate) fn wait(
        &self,
        key: &str,
        receiver: &mpsc::Receiver<Result<Value>>,
        timeout: Duration,
    ) -> Result<Value> {
        match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(waiter) = self.pending.lock().remove(key) {
                    waiter.timeout(timeout.as_millis().try_into().unwrap_or(u64::MAX));
                }
                Err(Error::timeout("Agent 没有在时限内响应"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(closed()),
        }
    }

    pub(crate) fn request(&self, key: String, frame: &Value, timeout: Duration) -> Result<Value> {
        self.request_then(key, frame, timeout, |_| {})
    }

    pub(crate) fn request_then(
        &self,
        key: String,
        frame: &Value,
        timeout: Duration,
        hook: impl FnOnce(&Value) + Send + 'static,
    ) -> Result<Value> {
        let _entered = self.process.trace.enter("agent.request.send");
        let receiver = self.expect_then(key.clone(), hook)?;
        if let Err(error) = self.process.send(frame) {
            self.forget(&key);
            return Err(error);
        }
        self.wait(&key, &receiver, timeout)
    }

    /// Hands a reply to its waiter; gives the result back when nobody is waiting for `key`.
    pub(crate) fn resolve(&self, key: &str, result: Result<Value>) -> Option<Result<Value>> {
        let waiter = self.pending.lock().remove(key);
        match waiter {
            Some(waiter) => {
                waiter.complete(result);
                None
            }
            None => Some(result),
        }
    }

    /// Drops the waiter for `key` (its command never reached the engine).
    pub(crate) fn forget(&self, key: &str) {
        if let Some(waiter) = self.pending.lock().remove(key) {
            waiter.cancel("not_sent");
            debug!("agent.request.not_sent");
        }
    }
}
