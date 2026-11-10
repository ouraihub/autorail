//! 主题 group (original `AppearanceSettings.tsx:342-466`): mode, fallback warnings, the fixed
//! theme list or the system pair, and contrast.

use std::sync::Arc;

use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Icon,
        button::Button,
        menu::{DropdownMenu as _, PopupMenuItem},
    },
    prelude::FluentBuilder as _,
};
use thinkrail_ui::{
    Appearance, Contrast, Roles, ThemeCatalog, ThinkrailTheme,
    components::{Segment, label},
    metrics::{radius, size, space, text},
};

use super::{
    super::rows::{disclosure, group, notice, row},
    AppearancePage,
};
use crate::{
    appearance::{
        AppearanceState, ModeChoice, ThemeView, select_contrast, select_fixed, select_mode,
        select_system,
    },
    strings::settings_appearance as s,
};

/// A theme's display name: the original's Chinese name for a bundled one, else its label.
pub fn theme_name(theme: &ThinkrailTheme) -> String {
    s::bundled_theme_name(&theme.id).map_or_else(|| theme.label.clone(), str::to_owned)
}

const MODES: [ModeChoice; 3] = [ModeChoice::System, ModeChoice::Light, ModeChoice::Dark];
const CONTRASTS: [Contrast; 2] = [Contrast::Normal, Contrast::High];

fn mode_index(mode: ModeChoice) -> usize {
    match mode {
        ModeChoice::System => 0,
        ModeChoice::Light => 1,
        ModeChoice::Dark => 2,
    }
}

impl AppearancePage {
    pub(super) fn theme_group(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let r = thinkrail_ui::ThinkrailTheme::global(cx).roles;
        let state = cx.global::<AppearanceState>();
        let (catalog, view, system) = (state.catalog.clone(), state.view(), state.system);
        let mode = Segment::new(
            "theme-mode",
            [s::MODE_SYSTEM, s::MODE_LIGHT, s::MODE_DARK],
            mode_index(view.mode()),
        )
        .on_select(cx.listener(|_, index: &usize, _, cx| {
            let Some(choice) = MODES.get(*index).copied() else {
                return;
            };
            let state = cx.global::<AppearanceState>();
            let next = select_mode(&state.catalog, &state.prefs, state.system, choice);
            Self::choose(next, cx);
        }));
        let contrast = Segment::new(
            "theme-contrast",
            [s::CONTRAST_DEFAULT, s::CONTRAST_HIGH],
            usize::from(view.contrast() == Contrast::High),
        )
        .on_select(cx.listener(|_, index: &usize, _, cx| {
            let Some(contrast) = CONTRASTS.get(*index).copied() else {
                return;
            };
            let state = cx.global::<AppearanceState>();
            let next = select_contrast(&state.catalog, &state.prefs, state.system, contrast);
            Self::choose(next, cx);
        }));
        let fallbacks = fallback_notices(&view, &r);
        let choice = if view.system_mode {
            self.system_rows(&catalog, &view, system, &r, cx)
        } else {
            self.fixed_rows(&catalog, &view, &r, cx)
        };
        group("appearance-theme", s::THEME_GROUP, None, &r)
            .child(row(s::THEME_MODE, None, mode, &r))
            .children(fallbacks)
            .children(choice)
            .child(row(s::CONTRAST, None, contrast, &r))
            .into_any_element()
    }

    fn fixed_rows(
        &self,
        catalog: &ThemeCatalog,
        view: &ThemeView,
        r: &Roles,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let open = self.themes_open;
        let mut rows = vec![disclosure(
            "theme-toggle",
            s::THEME,
            theme_name(&view.fixed.theme),
            open,
            r,
            {
                let page = cx.entity().downgrade();
                move |_, cx| {
                    if page
                        .update(cx, |page, cx| {
                            page.themes_open = !page.themes_open;
                            cx.notify();
                        })
                        .is_err()
                    {
                        tracing::debug!("appearance.theme.view_closed");
                    }
                }
            },
        )];
        if open {
            let active = view.fixed.theme.id.clone();
            let list = catalog.list().iter().map(|theme| {
                theme_option(
                    theme,
                    theme.id == active,
                    r,
                    cx.listener({
                        let id = theme.id.clone();
                        move |_, _, _, cx| {
                            let state = cx.global::<AppearanceState>();
                            let next = select_fixed(&state.prefs, &id);
                            Self::choose(next, cx);
                        }
                    }),
                )
            });
            rows.push(
                v_flex()
                    .gap(space::S4)
                    .py(space::S12)
                    .border_b_1()
                    .border_color(r.border_muted)
                    .children(list)
                    .into_any_element(),
            );
        }
        rows
    }

    fn system_rows(
        &self,
        catalog: &ThemeCatalog,
        view: &ThemeView,
        system: Appearance,
        r: &Roles,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let current = h_flex()
            .gap(space::S4)
            .items_center()
            .child(
                Icon::new(IconName::Monitor)
                    .size(size::ICON_SMALL)
                    .text_color(r.text_muted),
            )
            .child(label(
                text::UI,
                match system {
                    Appearance::Light => s::MODE_LIGHT,
                    Appearance::Dark => s::MODE_DARK,
                },
            ))
            .child(
                Icon::new(IconName::ArrowRight)
                    .size(size::ICON_SMALL)
                    .text_color(r.text_muted),
            )
            .child(
                Icon::new(IconName::Palette)
                    .size(size::ICON_SMALL)
                    .text_color(r.text_muted),
            )
            .child(label(text::UI, theme_name(&view.current.theme)));
        let open = self.themes_open;
        let summary = format!(
            "{} / {}",
            theme_name(&view.light.theme),
            theme_name(&view.dark.theme)
        );
        let mut rows = vec![
            row(s::CURRENT_ON_DEVICE, None, current, r),
            disclosure("theme-pair-toggle", s::THEME_PAIR, summary, open, r, {
                let page = cx.entity().downgrade();
                move |_, cx| {
                    if page
                        .update(cx, |page, cx| {
                            page.themes_open = !page.themes_open;
                            cx.notify();
                        })
                        .is_err()
                    {
                        tracing::debug!("appearance.theme.view_closed");
                    }
                }
            }),
        ];
        if open {
            for (appearance, title, resolved) in [
                (Appearance::Light, s::LIGHT_THEME, &view.light.theme),
                (Appearance::Dark, s::DARK_THEME, &view.dark.theme),
            ] {
                rows.push(row(
                    title,
                    None,
                    slot_menu(catalog, appearance, resolved),
                    r,
                ));
            }
        }
        rows
    }
}

fn fallback_notices(view: &ThemeView, r: &Roles) -> Vec<AnyElement> {
    let mut notices = Vec::new();
    if view.system_mode {
        if view.light.fallback {
            notices.push(notice(
                s::fallback_light(&theme_name(&view.light.theme)),
                true,
                r,
            ));
        }
        if view.dark.fallback {
            notices.push(notice(
                s::fallback_dark(&theme_name(&view.dark.theme)),
                true,
                r,
            ));
        }
    } else if view.fixed.fallback {
        notices.push(notice(
            s::fallback_fixed(&theme_name(&view.fixed.theme)),
            true,
            r,
        ));
    }
    notices
}

/// One row of the fixed theme list; the active one is highlighted and checked.
fn theme_option(
    theme: &ThinkrailTheme,
    active: bool,
    r: &Roles,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut gpui_kit::App) + 'static,
) -> AnyElement {
    h_flex()
        .id(format!("theme-option-{}", theme.id))
        .gap(space::S8)
        .px(space::S12)
        .py(space::S8)
        .rounded(radius::SM)
        .border_1()
        .cursor_pointer()
        .when(active, |d| {
            d.border_color(r.primary_muted)
                .bg(r.primary_subtle)
                .text_color(r.text_default)
        })
        .when(!active, |d| {
            d.border_color(r.border_default)
                .text_color(r.text_muted)
                .hover(|d| d.bg(r.control_bg_hovered).text_color(r.text_default))
        })
        .on_click(on_click)
        .child(label(text::UI, theme_name(theme)).flex_1())
        .when(active, |d| {
            d.child(
                Icon::new(IconName::Check)
                    .size(size::ICON)
                    .text_color(r.primary),
            )
        })
        .into_any_element()
}

/// A system slot's dropdown: the themes of that appearance.
fn slot_menu(
    catalog: &ThemeCatalog,
    appearance: Appearance,
    resolved: &Arc<ThinkrailTheme>,
) -> AnyElement {
    let themes: Vec<Arc<ThinkrailTheme>> = catalog
        .list()
        .iter()
        .filter(|t| t.appearance == appearance)
        .cloned()
        .collect();
    let id = match appearance {
        Appearance::Light => "theme-slot-light",
        Appearance::Dark => "theme-slot-dark",
    };
    Button::new(id)
        .outline()
        .label(theme_name(resolved))
        .icon(IconName::ChevronDown)
        .dropdown_menu(move |menu, _, _| {
            themes.iter().fold(menu, |menu, theme| {
                let theme_id = theme.id.clone();
                menu.item(
                    PopupMenuItem::new(theme_name(theme)).on_click(move |_, _, cx| {
                        let state = cx.global::<AppearanceState>();
                        let next = select_system(
                            &state.catalog,
                            &state.prefs,
                            state.system,
                            appearance,
                            &theme_id,
                        );
                        if let Some(next) = next {
                            AppearanceState::save(next, s::CHANGE_FAILED, cx);
                        }
                    }),
                )
            })
        })
        .into_any_element()
}
