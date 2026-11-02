//! Stateless, props-driven building blocks of the ThinkRail shell. They read only the installed
//! [`ThinkrailTheme`]; clicks are reported through the handlers they are given.

use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, base::h_flex, div,
    prelude::FluentBuilder as _, px,
};
use gpui_kit::{assets::IconName, component::Icon};

use crate::{
    fonts,
    metrics::{TextStyle, radius, size, space, text},
    theme::ThinkrailTheme,
};

type Handler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type SelectHandler = Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>;

/// Applies a `.tr-*` type style: family, nearest shipped face weight, size and line height.
pub trait TextStyled: Styled + Sized {
    fn text_style(self, style: TextStyle) -> Self {
        self.font_family(fonts::family(style.family))
            .font_weight(fonts::face_weight(&style))
            .text_size(style.size)
            .line_height(style.size * style.line_height)
    }
}

impl<T: Styled> TextStyled for T {}

/// Text with a type style, uppercased when the style says so.
pub fn label(style: TextStyle, content: impl Into<SharedString>) -> gpui_kit::Div {
    let content = content.into();
    let content: SharedString = if style.uppercase {
        content.to_uppercase().into()
    } else {
        content
    };
    div().text_style(style).child(content)
}

fn roles(cx: &App) -> crate::theme::Roles {
    ThinkrailTheme::global(cx).roles
}

/// A square icon button (24 px in headers). Disabled buttons paint muted and ignore clicks.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: IconName,
    selected: bool,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<Handler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName) -> Self {
        Self {
            id: id.into(),
            icon,
            selected: false,
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Accessible name, shown as the hover tooltip.
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let color = if self.disabled {
            r.control_disabled_text
        } else if self.selected {
            r.primary
        } else {
            r.text_muted
        };
        let tooltip = self.tooltip.clone();
        div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .size(size::HEADER_CONTROL)
            .flex_none()
            .rounded(radius::SM)
            .text_color(color)
            .when(self.selected, |d| d.bg(r.control_bg_selected))
            .when(!self.disabled, |d| {
                d.cursor_pointer().hover(|s| s.bg(r.control_bg_hovered))
            })
            .when_some(tooltip, |d, tip| {
                d.tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                })
            })
            .when_some(self.on_click.filter(|_| !self.disabled), |d, handler| {
                d.on_click(handler)
            })
            .child(Icon::new(self.icon).size(size::ICON).text_color(color))
    }
}

/// The 28 px header of a panel: a title on the left, action buttons on the right.
#[derive(IntoElement)]
pub struct PanelHeader {
    title: SharedString,
    actions: Vec<AnyElement>,
}

impl PanelHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            actions: Vec::new(),
        }
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }
}

impl RenderOnce for PanelHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        h_flex()
            .h(size::PANEL_HEADER)
            .flex_none()
            .px(size::PANEL_PADDING)
            .gap(space::S4)
            .border_b_1()
            .border_color(r.border_muted)
            .child(
                label(text::TITLE_COMPACT, self.title)
                    .flex_1()
                    .text_color(r.text_default)
                    .truncate(),
            )
            .children(self.actions)
    }
}

/// One 22 px row of a tree or list: optional chevron, icon, label, trailing slot.
#[derive(IntoElement)]
pub struct TreeRow {
    id: ElementId,
    depth: usize,
    icon: Option<IconName>,
    label: SharedString,
    detail: Option<SharedString>,
    expanded: Option<bool>,
    selected: bool,
    trailing: Vec<AnyElement>,
    on_click: Option<Handler>,
}

impl TreeRow {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            depth: 0,
            icon: None,
            label: label.into(),
            detail: None,
            expanded: None,
            selected: false,
            trailing: Vec::new(),
            on_click: None,
        }
    }

    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Secondary line (e.g. a workspace's branch), in `text-subtle`.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// `Some` shows a chevron (expandable row).
    pub fn expanded(mut self, expanded: Option<bool>) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing.push(element.into_any_element());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for TreeRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let name_color = if self.selected {
            r.primary
        } else {
            r.text_default
        };
        let indent = size::TREE_INDENT * self.depth as f32;
        let chevron = self.expanded.map(|open| {
            if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            }
        });
        let main = h_flex()
            .h(size::ROW)
            .gap(space::S4)
            .pl(indent + space::S4)
            .pr(space::S4)
            .child(
                div()
                    .w(size::ICON_SMALL)
                    .flex_none()
                    .when_some(chevron, |d, icon| {
                        d.child(
                            Icon::new(icon)
                                .size(size::ICON_SMALL)
                                .text_color(r.text_subtle),
                        )
                    }),
            )
            .when_some(self.icon, |d, icon| {
                d.child(Icon::new(icon).size(size::ICON).text_color(r.text_muted))
            })
            .child(
                label(text::UI, self.label)
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(name_color),
            )
            .children(self.trailing);
        let detail = self.detail.map(|detail| {
            let left = indent + space::S4 + size::ICON_SMALL + space::S4 + size::ICON + space::S4;
            label(text::METADATA, detail)
                .pl(left)
                .pb(space::S2)
                .truncate()
                .text_color(r.text_subtle)
        });
        div()
            .id(self.id)
            .flex()
            .flex_col()
            .rounded(radius::SM)
            .cursor_pointer()
            .when(self.selected, |d| d.bg(r.control_bg_selected))
            .when(!self.selected, |d| d.hover(|s| s.bg(r.control_bg_hovered)))
            .when_some(self.on_click, |d, handler| d.on_click(handler))
            .child(main)
            .children(detail)
    }
}

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
                        .rounded(radius::XS)
                        .hover(|s| s.bg(r.control_bg_hovered))
                        .on_click(handler)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Success,
    Warning,
    Error,
    Info,
    Idle,
}

/// An 8 px status dot plus optional label (e.g. the top bar's connection state).
#[derive(IntoElement)]
pub struct StatusDot {
    status: Status,
    label: Option<SharedString>,
}

impl StatusDot {
    pub fn new(status: Status) -> Self {
        Self {
            status,
            label: None,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

impl RenderOnce for StatusDot {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let color = match self.status {
            Status::Success => r.feedback_success,
            Status::Warning => r.feedback_warning,
            Status::Error => r.feedback_error,
            Status::Info => r.feedback_info,
            Status::Idle => r.text_subtle,
        };
        h_flex()
            .gap(space::S6)
            .child(
                div()
                    .size(size::STATUS_DOT)
                    .flex_none()
                    .rounded_full()
                    .bg(color),
            )
            .when_some(self.label, |d, text| {
                d.child(label(text::METADATA, text).text_color(r.text_muted))
            })
    }
}

/// A top-bar location selector: eyebrow caption above a value with a chevron.
#[derive(IntoElement)]
pub struct Breadcrumb {
    id: ElementId,
    eyebrow: SharedString,
    value: SharedString,
    icon: Option<IconName>,
    mono: bool,
    on_click: Option<Handler>,
}

impl Breadcrumb {
    pub fn new(
        id: impl Into<ElementId>,
        eyebrow: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            eyebrow: eyebrow.into(),
            value: value.into(),
            icon: None,
            mono: false,
            on_click: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Render the value in the code face (branch names).
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Breadcrumb {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let value_style = if self.mono {
            text::CODE_TEXT
        } else {
            text::ACTION
        };
        div()
            .id(self.id)
            .flex()
            .flex_col()
            .justify_center()
            .px(space::S8)
            .h_full()
            .rounded(radius::SM)
            .cursor_pointer()
            .hover(|s| s.bg(r.control_bg_hovered))
            .when_some(self.on_click, |d, handler| d.on_click(handler))
            .child(label(text::CAPTION, self.eyebrow).text_color(r.text_subtle))
            .child(
                h_flex()
                    .gap(space::S4)
                    .when_some(self.icon, |d, icon| {
                        d.child(
                            Icon::new(icon)
                                .size(size::ICON_SMALL)
                                .text_color(r.text_muted),
                        )
                    })
                    .child(
                        label(value_style, self.value)
                            .text_color(r.text_default)
                            .truncate(),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .size(size::ICON_SMALL)
                            .text_color(r.text_subtle),
                    ),
            )
    }
}

/// One activity-bar button: `control-bg-selected` fill and a `primary` left bar when active.
#[derive(IntoElement)]
pub struct ActivityItem {
    id: ElementId,
    icon: IconName,
    tooltip: SharedString,
    active: bool,
    on_click: Option<Handler>,
}

impl ActivityItem {
    pub fn new(id: impl Into<ElementId>, icon: IconName, tooltip: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon,
            tooltip: tooltip.into(),
            active: false,
            on_click: None,
        }
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
}

impl RenderOnce for ActivityItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let r = roles(cx);
        let color = if self.active {
            r.text_default
        } else {
            r.text_muted
        };
        let tooltip = self.tooltip;
        div()
            .id(self.id)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .w(size::ACTIVITY_BAR)
            .h(size::ACTIVITY_BAR)
            .cursor_pointer()
            .text_color(color)
            .when(self.active, |d| d.bg(r.control_bg_selected))
            .when(!self.active, |d| d.hover(|s| s.bg(r.control_bg_hovered)))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
            })
            .when_some(self.on_click, |d, handler| d.on_click(handler))
            .when(self.active, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(size::ACCENT_BAR)
                        .bg(r.primary),
                )
            })
            .child(Icon::new(self.icon).size(size::ICON).text_color(color))
    }
}

/// Components hold closures and child elements, which have no `Debug`; name the type only.
macro_rules! opaque_debug {
    ($($ty:ident),* $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )*};
}

opaque_debug!(
    IconButton,
    PanelHeader,
    TreeRow,
    Tab,
    TabStrip,
    Segment,
    StatusDot,
    Breadcrumb,
    ActivityItem
);
