//! Saved-config model discovery; picker editing and writes live in `picker`.

use gpui_kit::{Context, Window};
use thinkrail_core::ProbeTarget;

use super::super::ProvidersPage;
use crate::state::run;

impl ProvidersPage {
    pub(super) fn sync_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self.provider().cloned() else {
            return;
        };
        let Some(endpoint) = provider
            .input
            .endpoints
            .iter()
            .find(|e| e.protocol == self.line)
            .cloned()
        else {
            return;
        };
        self.models.syncing = true;
        self.models.sync_error = None;
        let (service, probe) = (self.service(cx), self.state.read(cx).services.probe.clone());
        let (line, selected) = (self.line, self.selected.clone());
        let handle = window.window_handle();
        run(
            cx,
            move || {
                let key = provider
                    .keys
                    .iter()
                    .find(|k| k.enabled)
                    .map(|k| service.reveal_key(&k.id))
                    .transpose()?;
                probe.list_models(&ProbeTarget {
                    endpoint,
                    headers: provider.input.headers.clone(),
                    key,
                })
            },
            move |this, result, cx| {
                if this.selected != selected || this.line != line {
                    return;
                }
                this.models.syncing = false;
                match result {
                    Ok(found) => {
                        let page = cx.entity();
                        let draft = this
                            .draft
                            .as_ref()
                            .map(|d| d.models.clone())
                            .unwrap_or_default();
                        let _ = handle.update(cx, |_, window, cx| {
                            super::picker::open_picker(page, found, draft, line, window, cx)
                        });
                    }
                    Err(error) => this.models.sync_error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
    }
}
