//! The font picker (original `FontPicker.tsx`, `fontCatalog.ts`; 08 A12, A18), opened inline
//! under the row it edits: a search field and the 内置 / 自定义 / 已安装 groups, a code role lists
//! monospaced families only, and a typed name that is not listed can be used as is. Fallbacks
//! list neither custom fonts (DirectWrite does not fall back to them) nor families already taken.

use std::collections::HashSet;

use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window,
    base::{h_flex, v_flex},
    component::{
        Sizable as _,
        input::{Escape, Input},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{FONT_FALLBACKS_MAX, FontFamily, FontRole, is_font_family_name};
use thinkrail_ui::{
    Roles, ThinkrailTheme,
    components::label,
    fonts,
    metrics::{radius, space, text},
};

use super::{AppearancePage, PickerTarget};
use crate::{
    appearance::{AppearanceState, bundled_face, family_name},
    strings::settings_appearance as s,
};

/// Rows shown at most per group; the search narrows the rest.
const MAX_ROWS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Builtin,
    Custom,
    Installed,
}

struct Option_ {
    family: FontFamily,
    label: String,
    group: Group,
    monospace: bool,
}

impl AppearancePage {
    pub(super) fn toggle_picker(
        &mut self,
        target: PickerTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.picker == Some(target) {
            self.picker = None;
        } else {
            self.picker = Some(target);
            self.search.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn pick(&mut self, target: PickerTarget, family: FontFamily, cx: &mut Context<Self>) {
        self.picker = None;
        AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| match target {
            PickerTarget::Family(role) => {
                prefs.font_mut(role).family = match family {
                    FontFamily::Builtin(builtin) if builtin == role => None,
                    other => Some(other),
                };
            }
            PickerTarget::Fallback(role) => {
                let fallbacks = &mut prefs.font_mut(role).fallbacks;
                if fallbacks.len() < FONT_FALLBACKS_MAX && !fallbacks.contains(&family) {
                    fallbacks.push(family);
                }
            }
        });
    }

    /// The picker panel when `target` is the open one.
    pub(super) fn picker_for(
        &mut self,
        target: PickerTarget,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.picker != Some(target) {
            return None;
        }
        let r = ThinkrailTheme::global(cx).roles;
        let query = self.search.read(cx).value().trim().to_owned();
        let options = self.options(target, &query, cx);
        let lower = query.to_lowercase();
        let listed = options
            .iter()
            .any(|o| family_name(&o.family).to_lowercase() == lower);
        let typed = (!query.is_empty() && !listed && is_font_family_name(&query)).then_some(query);
        let mono_only = matches!(target, PickerTarget::Family(FontRole::Code));
        let mut body: Vec<AnyElement> = Vec::new();
        for (group, title) in [
            (Group::Builtin, s::PICKER_BUILTIN),
            (Group::Custom, s::PICKER_CUSTOM),
            (Group::Installed, s::PICKER_INSTALLED),
        ] {
            let rows: Vec<_> = options.iter().filter(|o| o.group == group).collect();
            if rows.is_empty() {
                continue;
            }
            body.push(
                label(text::EYEBROW, title)
                    .text_color(r.text_subtle)
                    .pt(space::S8)
                    .into_any_element(),
            );
            for option in rows {
                body.push(self.option_row(target, option, &r, cx));
            }
        }
        if let Some(name) = typed {
            let family = FontFamily::Name(name.clone());
            body.push(
                h_flex()
                    .id("font-picker-use")
                    .px(space::S8)
                    .py(space::S4)
                    .rounded(radius::SM)
                    .cursor_pointer()
                    .hover(|d| d.bg(r.control_bg_hovered))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pick(target, family.clone(), cx);
                    }))
                    .child(label(text::UI, s::picker_use(&name)).text_color(r.primary))
                    .into_any_element(),
            );
        }
        if body.is_empty() {
            body.push(
                label(text::METADATA, s::PICKER_EMPTY)
                    .text_color(r.text_subtle)
                    .p(space::S8)
                    .into_any_element(),
            );
        }
        Some(
            v_flex()
                .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                    this.picker = None;
                    cx.stop_propagation();
                    cx.notify();
                }))
                .gap(space::S4)
                .my(space::S8)
                .p(space::S8)
                .rounded(radius::MD)
                .border_1()
                .border_color(r.border_default)
                .bg(r.container_elevated_bg)
                .child(Input::new(&self.search).small())
                .child(
                    div()
                        .id("font-picker-list")
                        .max_h(px(280.))
                        .overflow_y_scroll()
                        .child(v_flex().children(body)),
                )
                .when(mono_only, |d| {
                    d.child(label(text::METADATA, s::PICKER_MONO_ONLY).text_color(r.text_subtle))
                })
                .into_any_element(),
        )
    }

    fn option_row(
        &self,
        target: PickerTarget,
        option: &Option_,
        r: &Roles,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let family = option.family.clone();
        let id = format!("font-option-{}", option.label);
        h_flex()
            .id(gpui_kit::SharedString::from(id))
            .gap(space::S8)
            .px(space::S8)
            .py(space::S4)
            .rounded(radius::SM)
            .cursor_pointer()
            .hover(|d| d.bg(r.control_bg_hovered))
            .on_click(cx.listener(move |this, _, _, cx| this.pick(target, family.clone(), cx)))
            .child(label(text::UI, option.label.clone()).flex_1())
            .when(option.monospace, |d| {
                d.child(label(text::CAPTION, s::PICKER_MONO).text_color(r.text_subtle))
            })
            .into_any_element()
    }

    /// The families this target can take, filtered by the search text (original
    /// `fontOptions` / `mainFontOptions` / `fallbackFontOptions`).
    fn options(&self, target: PickerTarget, query: &str, cx: &Context<Self>) -> Vec<Option_> {
        let state = cx.global::<AppearanceState>();
        let (role, fallback) = match target {
            PickerTarget::Family(role) => (role, false),
            PickerTarget::Fallback(role) => (role, true),
        };
        let choice = state.prefs.font(role);
        // A fallback cannot repeat the primary or another fallback.
        let taken: HashSet<String> = choice
            .family
            .iter()
            .chain(choice.fallbacks.iter())
            .map(|f| family_name(f).to_lowercase())
            .chain(fallback.then(|| bundled_face(role).to_lowercase()))
            .collect();
        let lower = query.to_lowercase();
        let matches = |name: &str| lower.is_empty() || name.to_lowercase().contains(&lower);
        let mono_only = !fallback && role == FontRole::Code;
        let mut out = Vec::new();
        for builtin in [FontRole::Interface, FontRole::Code] {
            let name = bundled_face(builtin);
            let monospace = builtin == FontRole::Code;
            if (mono_only && !monospace) || taken.contains(&name.to_lowercase()) || !matches(name) {
                continue;
            }
            out.push(Option_ {
                family: FontFamily::Builtin(builtin),
                label: name.to_owned(),
                group: Group::Builtin,
                monospace,
            });
        }
        let mut seen: HashSet<String> = [fonts::INTERFACE, fonts::CODE]
            .iter()
            .map(|n| n.to_lowercase())
            .collect();
        let mut custom: Vec<(String, bool)> = state
            .fonts
            .iter()
            .filter_map(|f| f.face.as_ref().ok())
            .map(|face| (face.family.clone(), face.monospace))
            .collect();
        custom.sort_by_key(|(name, _)| name.to_lowercase());
        for (name, monospace) in custom {
            let key = name.to_lowercase();
            if !seen.insert(key.clone()) {
                continue;
            }
            // DirectWrite never falls back to a registered font (08 A18).
            if fallback || (mono_only && !monospace) || taken.contains(&key) || !matches(&name) {
                continue;
            }
            out.push(Option_ {
                family: FontFamily::Name(name.clone()),
                label: name,
                group: Group::Custom,
                monospace,
            });
        }
        let installed = std::sync::Arc::clone(&state.installed);
        let mut shown = 0;
        for name in installed.iter() {
            if shown >= MAX_ROWS {
                break;
            }
            let key = name.to_lowercase();
            if seen.contains(&key) || taken.contains(&key) || !matches(name) {
                continue;
            }
            let monospace = fonts::is_monospace(cx, name);
            if mono_only && !monospace {
                continue;
            }
            shown += 1;
            out.push(Option_ {
                family: FontFamily::Name(name.clone()),
                label: name.clone(),
                group: Group::Installed,
                monospace,
            });
        }
        out
    }
}
