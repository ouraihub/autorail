//! A failure reported from a background job carries that job's TraceId to the notice bar.

use gpui_kit::{Context, Entity, IntoElement, Render, TestAppContext, Window, div};
use thinkrail_core::Error;

use super::{AppState, run};
use crate::testing;

struct Host {
    state: Entity<AppState>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui_kit::test]
fn a_failed_background_job_leaves_its_trace_id_on_the_notice(cx: &mut TestAppContext) {
    let (_fixture, host) = testing::open(cx, |state, _, _| Host { state });
    let state = cx.update(|cx| host.read(cx).state.clone());
    cx.run_until_parked();
    cx.update(|cx| {
        state.update(cx, |_, cx| {
            run(
                cx,
                || Err::<(), _>(Error::invalid("boom")),
                |state: &mut AppState, result, cx| {
                    if let Err(error) = result {
                        state.report(&error, cx);
                    }
                },
            )
        })
    });
    cx.run_until_parked();
    let notice = cx
        .update(|cx| state.read(cx).notice.clone())
        .expect("notice shown");
    let trace_id = notice.trace_id.expect("background failure has a TraceId");
    assert_eq!(trace_id.len(), 32);
    assert!(trace_id.bytes().all(|b| b.is_ascii_hexdigit()));

    // Reported outside a background completion: no TraceId to point at.
    cx.update(|cx| state.update(cx, |state, cx| state.report(&Error::invalid("direct"), cx)));
    let direct = cx
        .update(|cx| state.read(cx).notice.clone())
        .expect("notice shown");
    assert_eq!(direct.trace_id, None);
}
