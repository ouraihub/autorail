//! 服务商栏：搜索、筛选、服务商行（头像、名称、状态点、⋯ 菜单）、新建。

use gpui_kit::{
    AnyElement, Context, IntoElement, Styled as _,
    assets::IconName,
    component::{
        Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        input::Input,
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    prelude::FluentBuilder as _,
};
use lumakit_gpui::providers as ui;
use thinkrail_core::Provider;

use super::{Filter, ProvidersPage, theme};
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
        let r = theme::roles(cx);
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
            vec![ui::empty(s::NO_MATCHES, &r)]
        } else {
            visible.iter().map(|p| self.render_row(p, cx)).collect()
        };
        ui::rail(
            Input::new(&self.search)
                .prefix(Icon::new(IconName::Search).small())
                .into_any_element(),
            filter_menu.into_any_element(),
            rows,
            Button::new("providers-add")
                .w_full()
                .icon(IconName::Plus)
                .label(s::ADD)
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_name_dialog(None, window, cx)),
                )
                .into_any_element(),
        )
        .into_any_element()
    }

    fn render_row(&self, provider: &Provider, cx: &mut Context<Self>) -> AnyElement {
        let r = theme::roles(cx);
        let id = provider.id.clone();
        let current = self.selected.as_ref() == Some(&id);
        let enabled = provider.input.enabled;
        let has_key = provider.keys.iter().any(|k| k.enabled);
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
        ui::provider_row(
            format!("provider-row-{id}"),
            provider.input.name.clone(),
            current,
            enabled,
            has_key,
            menu.into_any_element(),
            &r,
        )
        .on_select({
            let page = cx.entity();
            move |_, cx| {
                page.update(cx, |this, cx| {
                    if this.selected.as_ref() != Some(&id) {
                        this.select(id.clone(), cx);
                    }
                });
            }
        })
        .into_any_element()
    }
}
