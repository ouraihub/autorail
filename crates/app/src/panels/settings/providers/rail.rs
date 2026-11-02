//! 服务商栏：搜索、筛选、服务商行（头像、名称、状态点、⋯ 菜单）、新建。

use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::Provider;
use thinkrail_ui::{
    ThinkrailTheme,
    components::label,
    metrics::{radius, size, space, text},
};

use super::{Filter, ProvidersPage, avatar};
use crate::strings::settings_providers as s;

impl ProvidersPage {
    fn visible(&self, cx: &Context<Self>) -> Vec<Provider> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.providers
            .iter()
            .filter(|p| match self.filter {
                Filter::All => true,
                Filter::Enabled => p.input.enabled,
                Filter::Disabled => !p.input.enabled,
            })
            .filter(|p| query.is_empty() || p.input.name.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    pub(super) fn render_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let page = cx.entity();
        let filter = self.filter;
        let filter_menu = Button::new("providers-filter")
            .outline()
            .icon(IconName::SlidersHorizontal)
            .tooltip(s::FILTER)
            .when(filter != Filter::All, |b| {
                gpui_kit::component::Selectable::selected(b, true)
            })
            .dropdown_menu(move |menu, _, _| {
                [
                    (Filter::All, s::FILTER_ALL),
                    (Filter::Enabled, s::FILTER_ENABLED),
                    (Filter::Disabled, s::FILTER_DISABLED),
                ]
                .into_iter()
                .fold(menu, |menu, (value, title)| {
                    let page = page.clone();
                    menu.item(PopupMenuItem::new(title).checked(filter == value).on_click(
                        move |_, _, cx| {
                            page.update(cx, |this, cx| {
                                this.filter = value;
                                cx.notify();
                            });
                        },
                    ))
                })
            });
        let visible = self.visible(cx);
        let rows: Vec<AnyElement> = if visible.is_empty() && self.loaded {
            vec![
                label(text::METADATA, s::NO_MATCHES)
                    .text_color(r.text_subtle)
                    .p(space::S8)
                    .into_any_element(),
            ]
        } else {
            visible.iter().map(|p| self.render_row(p, cx)).collect()
        };
        v_flex()
            .w(size::PROVIDER_LIST)
            .flex_none()
            .gap(space::S4)
            .child(
                h_flex()
                    .gap(space::S6)
                    .child(
                        Input::new(&self.search)
                            .prefix(Icon::new(IconName::Search).small())
                            .flex_1(),
                    )
                    .child(filter_menu),
            )
            .children(rows)
            .child(
                Button::new("providers-add")
                    .w_full()
                    .icon(IconName::Plus)
                    .label(s::ADD)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_name_dialog(None, window, cx)),
                    ),
            )
            .into_any_element()
    }

    fn render_row(&self, provider: &Provider, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let id = provider.id.clone();
        let current = self.selected.as_ref() == Some(&id);
        let enabled = provider.input.enabled;
        let has_key = provider.keys.iter().any(|k| k.enabled);
        let dot = enabled.then(|| {
            if has_key {
                r.feedback_success
            } else {
                r.feedback_warning
            }
        });
        let page = cx.entity();
        let menu_target = provider.clone();
        let menu = Button::new(format!("provider-menu-{id}"))
            .ghost()
            .xsmall()
            .icon(IconName::Ellipsis)
            .tooltip(s::ROW_MENU)
            .dropdown_menu(move |menu, _, _| {
                let (edit, delete, page_edit, page_delete) = (
                    menu_target.clone(),
                    menu_target.clone(),
                    page.clone(),
                    page.clone(),
                );
                menu.item(PopupMenuItem::new(s::EDIT).icon(IconName::Pencil).on_click(
                    move |_, window, cx| {
                        page_edit.update(cx, |this, cx| {
                            this.open_name_dialog(Some(edit.clone()), window, cx)
                        });
                    },
                ))
                .item(
                    PopupMenuItem::new(s::DELETE)
                        .icon(IconName::Trash)
                        .disabled(menu_target.preset_id.is_some())
                        .on_click(move |_, window, cx| {
                            page_delete.update(cx, |this, cx| {
                                this.confirm_delete(delete.clone(), window, cx)
                            })
                        }),
                )
            });
        h_flex()
            .id(format!("provider-row-{id}"))
            .h(px(36.))
            .px(space::S8)
            .gap(space::S8)
            .rounded(radius::MD)
            .cursor_pointer()
            .when(current, |d| d.bg(r.control_bg_selected))
            .when(!current, |d| d.hover(|s| s.bg(r.control_bg_hovered)))
            .on_click(cx.listener(move |this, _, _, cx| {
                if this.selected.as_ref() != Some(&id) {
                    this.select(id.clone(), cx)
                }
            }))
            .child(avatar(&provider.input.name, cx))
            .child(
                label(text::UI, provider.input.name.clone())
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(if enabled {
                        r.text_default
                    } else {
                        r.text_muted
                    }),
            )
            .children(dot.map(|color| div().size(px(6.)).flex_none().rounded_full().bg(color)))
            .child(menu)
            .into_any_element()
    }
}
