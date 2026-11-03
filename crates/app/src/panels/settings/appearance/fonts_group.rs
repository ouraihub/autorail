//! 字体 group (original `FontsSettings.tsx:344-591`): the preview, the interface font and text
//! weight, the code font, size and ligatures, and the fallback fonts under 高级.

use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        slider::Slider,
        switch::Switch,
    },
    prelude::FluentBuilder as _,
};
use thinkrail_core::{CodeFontSize, FontChoice, FontRole, FontWeightOffset};
use thinkrail_ui::{
    Roles, ThinkrailTheme,
    components::label,
    metrics::{radius, space, text},
};

use super::{
    AppearancePage, PickerTarget, preview,
    rows::{disclosure, group, notice, row, subheading, with_reset},
};
use crate::{
    appearance::{AppearanceState, bundled_face, family_name, role_font},
    strings::settings_appearance as s,
};

fn weight_text(offset: i16) -> String {
    match offset {
        0 => s::CONTRAST_DEFAULT.to_owned(),
        o if o > 0 => format!("+{o}"),
        o => format!("−{}", -o),
    }
}

impl AppearancePage {
    pub(super) fn fonts_group(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let state = cx.global::<AppearanceState>();
        let prefs = state.prefs.clone();
        let interface = role_font(&prefs.interface_font, FontRole::Interface, &state.families);
        let code = role_font(&prefs.code_font, FontRole::Code, &state.families);
        let static_faces: Vec<&str> = [&interface.family, &code.family]
            .into_iter()
            .filter(|f| {
                *f == bundled_face(FontRole::Interface) || *f == bundled_face(FontRole::Code)
            })
            .map(String::as_str)
            .collect();
        let mut body = vec![
            preview::card(prefs.code_font_size, &r),
            subheading(s::INTERFACE_TITLE, s::INTERFACE_DESCRIPTION, &r),
            self.family_row(FontRole::Interface, &prefs.interface_font, &r, cx),
        ];
        body.extend(self.picker_for(PickerTarget::Family(FontRole::Interface), window, cx));
        body.extend(interface.unavailable.as_deref().map(|name| {
            notice(
                s::unavailable(name, bundled_face(FontRole::Interface)),
                true,
                &r,
            )
        }));
        body.push(self.weight_row(prefs.font_weight_offset, &static_faces, &r, cx));
        body.push(subheading(s::CODE_TITLE, s::CODE_DESCRIPTION, &r));
        body.push(self.family_row(FontRole::Code, &prefs.code_font, &r, cx));
        body.extend(self.picker_for(PickerTarget::Family(FontRole::Code), window, cx));
        body.extend(
            code.unavailable
                .as_deref()
                .map(|name| notice(s::unavailable(name, bundled_face(FontRole::Code)), true, &r)),
        );
        body.push(self.size_row(prefs.code_font_size, &r));
        body.push(self.ligatures_row(prefs.code_ligatures, &r, cx));
        let open = self.advanced_open;
        body.push(disclosure(
            "font-advanced-toggle",
            s::ADVANCED,
            s::ADVANCED_SUMMARY.to_owned(),
            open,
            &r,
            {
                let page = cx.entity().downgrade();
                move |_, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.advanced_open = !page.advanced_open;
                        cx.notify();
                    });
                }
            },
        ));
        if open {
            for role in [FontRole::Interface, FontRole::Code] {
                body.push(self.fallback_row(role, prefs.font(role), &r, cx));
                body.extend(self.picker_for(PickerTarget::Fallback(role), window, cx));
            }
        }
        group(
            "appearance-fonts",
            s::FONTS_TITLE,
            Some(s::FONTS_DESCRIPTION),
            &r,
        )
        .gap(space::S4)
        .child(v_flex().pt(space::S12).children(body))
        .into_any_element()
    }

    fn family_row(
        &self,
        role: FontRole,
        choice: &FontChoice,
        r: &Roles,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shown = choice
            .family
            .as_ref()
            .map_or(bundled_face(role), family_name)
            .to_owned();
        let (key, reset_label) = match role {
            FontRole::Interface => ("font-interface", s::RESET_INTERFACE_FONT),
            FontRole::Code => ("font-code", s::RESET_CODE_FONT),
        };
        let trigger = Button::new(key)
            .outline()
            .label(shown)
            .icon(IconName::ChevronDown)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_picker(PickerTarget::Family(role), window, cx);
            }));
        let control = with_reset(
            match role {
                FontRole::Interface => "font-interface-reset",
                FontRole::Code => "font-code-reset",
            },
            choice.family.is_some(),
            reset_label,
            trigger,
            move |_, cx| {
                AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| {
                    prefs.font_mut(role).family = None;
                });
            },
        );
        row(s::FONT, None, control, r)
    }

    fn weight_row(
        &self,
        offset: i16,
        static_faces: &[&str],
        r: &Roles,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        let reset = with_reset(
            "font-weight-reset",
            offset != FontWeightOffset::DEFAULT,
            s::RESET_WEIGHT,
            label(text::UI, weight_text(offset)),
            |_, cx| {
                AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| {
                    prefs.font_weight_offset = FontWeightOffset::DEFAULT;
                });
            },
        );
        v_flex()
            .gap(space::S8)
            .py(space::S12)
            .border_b_1()
            .border_color(r.border_muted)
            .child(
                h_flex()
                    .items_start()
                    .gap(space::S12)
                    .child(
                        v_flex()
                            .flex_1()
                            .gap(space::S2)
                            .child(label(text::UI, s::WEIGHT))
                            .child(
                                label(text::METADATA, s::WEIGHT_DESCRIPTION)
                                    .text_color(r.text_muted),
                            ),
                    )
                    .child(reset),
            )
            .child(Slider::new(&self.weight).horizontal())
            .child(
                h_flex()
                    .justify_between()
                    .child(label(text::METADATA, s::WEIGHT_LIGHTER).text_color(r.text_muted))
                    .child(label(text::METADATA, s::WEIGHT_HEAVIER).text_color(r.text_muted)),
            )
            .children(
                static_faces
                    .iter()
                    .map(|family| notice(s::weight_steps(family), false, r)),
            )
            .into_any_element()
    }

    fn size_row(&self, size: u8, r: &Roles) -> AnyElement {
        let step = |delta: i8| {
            move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut gpui_kit::App| {
                AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| {
                    prefs.code_font_size = prefs
                        .code_font_size
                        .saturating_add_signed(delta)
                        .clamp(CodeFontSize::MIN, CodeFontSize::MAX);
                });
            }
        };
        let stepper = h_flex()
            .gap(space::S4)
            .items_center()
            .child(
                Button::new("font-size-smaller")
                    .outline()
                    .small()
                    .icon(IconName::Minus)
                    .tooltip(s::SMALLER)
                    .accessibility_label(s::SMALLER)
                    .disabled(size <= CodeFontSize::MIN)
                    .on_click(step(-1)),
            )
            .child(
                label(text::UI, format!("{size} px"))
                    .min_w(gpui_kit::px(48.))
                    .text_center()
                    .px(space::S8)
                    .py(space::S4)
                    .rounded(radius::SM)
                    .border_1()
                    .border_color(r.control_border_default),
            )
            .child(
                Button::new("font-size-larger")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip(s::LARGER)
                    .accessibility_label(s::LARGER)
                    .disabled(size >= CodeFontSize::MAX)
                    .on_click(step(1)),
            );
        row(
            s::SIZE,
            Some(s::size_description(CodeFontSize::EDITOR_DELTA)),
            with_reset(
                "font-size-reset",
                size != CodeFontSize::DEFAULT,
                s::RESET_SIZE,
                stepper,
                |_, cx| {
                    AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| {
                        prefs.code_font_size = CodeFontSize::DEFAULT;
                    });
                },
            ),
            r,
        )
    }

    fn ligatures_row(&self, on: bool, r: &Roles, cx: &mut Context<Self>) -> AnyElement {
        let switch = Switch::new("font-ligatures")
            .checked(on)
            .accessibility_label(s::LIGATURES)
            .on_click(cx.listener(|_, checked: &bool, _, cx| {
                let checked = *checked;
                AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| {
                    prefs.code_ligatures = checked;
                });
            }));
        row(
            s::LIGATURES,
            Some(s::LIGATURES_DESCRIPTION.to_owned()),
            with_reset(
                "font-ligatures-reset",
                on,
                s::RESET_LIGATURES,
                switch,
                |_, cx| {
                    AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| {
                        prefs.code_ligatures = false;
                    });
                },
            ),
            r,
        )
    }

    fn fallback_row(
        &self,
        role: FontRole,
        choice: &FontChoice,
        r: &Roles,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, description) = match role {
            FontRole::Interface => (s::INTERFACE_FALLBACKS, s::INTERFACE_FALLBACKS_DESCRIPTION),
            FontRole::Code => (s::CODE_FALLBACKS, s::CODE_FALLBACKS_DESCRIPTION),
        };
        let chips = choice
            .fallbacks
            .iter()
            .enumerate()
            .map(|(index, fallback)| {
                h_flex()
                    .gap(space::S4)
                    .items_center()
                    .pl(space::S8)
                    .rounded(radius::SM)
                    .border_1()
                    .border_color(r.border_default)
                    .child(label(text::METADATA, family_name(fallback).to_owned()))
                    .child(
                        Button::new(format!("font-{role:?}-fallback-remove-{index}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .on_click(move |_, _, cx| {
                                AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| {
                                    let fallbacks = &mut prefs.font_mut(role).fallbacks;
                                    if index < fallbacks.len() {
                                        fallbacks.remove(index);
                                    }
                                });
                            }),
                    )
            });
        let full = choice.fallbacks.len() >= thinkrail_core::FONT_FALLBACKS_MAX;
        let control = h_flex()
            .flex_wrap()
            .gap(space::S4)
            .max_w(gpui_kit::px(360.))
            .children(chips)
            .when(!full, |d| {
                d.child(
                    Button::new(match role {
                        FontRole::Interface => "font-interface-fallback-add",
                        FontRole::Code => "font-code-fallback-add",
                    })
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .label(s::ADD_FALLBACK)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.toggle_picker(PickerTarget::Fallback(role), window, cx);
                    })),
                )
            });
        row(title, Some(description.to_owned()), control, r)
    }
}
