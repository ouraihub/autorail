//! One-of-many selectors: tabs, the tab strip and the segmented choice.

use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, TestSupportExt as _, Window,
    base::h_flex, div, prelude::FluentBuilder as _, px,
};
use gpui_kit::{assets::IconName, component::Icon};

use super::{Handler, SelectHandler, label, roles};
use crate::metrics::{radius, size, space, text};

/// One tab of a [`TabStrip`].
#[derive(IntoElement)]
pub struct Tab {
    id: ElementId,
    icon: Option<IconName>,
    label: SharedString,
    active: bool,
    on_click: Option<Handler>,
    on_close: Option<Handler>,
}

impl Tab {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            active: false,
            on_click: None,
            on_close: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    pub fn on_close(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Tab {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let color = if self.active {
            r.text_default
        } else {
            r.text_muted
        };
        let close_id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "close".into());
        h_flex()
            .id(self.id)
            .test_support()
            .h_full()
            .gap(space::S6)
            .px(space::S8)
            .flex_none()
            .cursor_pointer()
            .border_r_1()
            .border_color(r.border_muted)
            .text_color(color)
            .when(self.active, |d| d.bg(r.container_workspace_bg))
            .when(!self.active, |d| d.hover(|s| s.bg(r.control_bg_hovered)))
            .when_some(self.on_click, |d, handler| d.on_click(handler))
            .when_some(self.icon, |d, icon| {
                d.child(Icon::new(icon).size(size::ICON_SMALL))
            })
            .child(label(text::ACTION, self.label))
            .when_some(self.on_close, |d, handler| {
                d.child(
                    div()
                        .id(close_id)
                        .test_support()
                        .rounded(radius::XS)
                        .hover(|s| s.bg(r.control_bg_hovered))
                        // The tab itself is clickable: without this the click also selects the
                        // tab being closed.
                        .on_click(move |event, window, cx| {
                            cx.stop_propagation();
                            handler(event, window, cx);
                        })
                        .child(
                            Icon::new(IconName::X)
                                .size(size::ICON_SMALL)
                                .text_color(r.text_subtle),
                        ),
                )
            })
    }
}

/// The 28 px tab row with trailing actions.
#[derive(IntoElement)]
pub struct TabStrip {
    tabs: Vec<AnyElement>,
    actions: Vec<AnyElement>,
}

impl TabStrip {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            actions: Vec::new(),
        }
    }

    pub fn tab(mut self, tab: Tab) -> Self {
        self.tabs.push(tab.into_any_element());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl Default for TabStrip {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for TabStrip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        h_flex()
            .h(size::TAB_STRIP)
            .flex_none()
            .bg(r.container_header_bg)
            .border_b_1()
            .border_color(r.border_muted)
            .child(
                h_flex()
                    .h_full()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .children(self.tabs),
            )
            .child(h_flex().gap(space::S2).px(space::S4).children(self.actions))
    }
}

/// A segmented choice (e.g. List / Tree). Exactly one option is active.
#[derive(IntoElement)]
pub struct Segment {
    id: ElementId,
    options: Vec<SharedString>,
    active: usize,
    on_select: Option<SelectHandler>,
}

impl Segment {
    pub fn new(
        id: impl Into<ElementId>,
        options: impl IntoIterator<Item = impl Into<SharedString>>,
        active: usize,
    ) -> Self {
        Self {
            id: id.into(),
            options: options.into_iter().map(Into::into).collect(),
            active,
            on_select: None,
        }
    }

    pub fn on_select(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Segment {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let handler = self.on_select.map(std::rc::Rc::new);
        let base = std::sync::Arc::new(self.id);
        h_flex()
            .h(size::HEADER_CONTROL)
            .p(space::S2)
            .gap(space::S2)
            .rounded(radius::SM)
            .bg(r.control_bg)
            .children(self.options.into_iter().enumerate().map(|(index, option)| {
                let active = index == self.active;
                let handler = handler.clone();
                div()
                    .id(ElementId::NamedChild(
                        base.clone(),
                        index.to_string().into(),
                    ))
                    .h_full()
                    .flex()
                    .items_center()
                    .px(space::S6)
                    .rounded(radius::XS)
                    .cursor_pointer()
                    .text_color(if active { r.text_default } else { r.text_muted })
                    .when(active, |d| d.bg(r.control_bg_selected))
                    .when(!active, |d| d.hover(|s| s.bg(r.control_bg_hovered)))
                    .when_some(handler, move |d, handler| {
                        d.on_click(move |_, window, cx| handler(&index, window, cx))
                    })
                    .child(label(text::ACTION, option))
            }))
    }
}

#[cfg(test)]
mod tests;
