//! 设置 → 界面 → 外观 (native-multi-agent 08): the original's Appearance page. The values live
//! in the [`AppearanceState`] global; this page holds only what is open, typed or being dragged,
//! and re-renders whenever the global changes.

mod custom;
mod fonts_group;
mod picker;
mod preview;
mod rows;
mod theme_group;

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Subscription, Window,
    base::v_flex,
    component::{
        input::InputState,
        slider::{SliderEvent, SliderState},
    },
};
use thinkrail_core::{AppearancePrefs, FontRole, FontWeightOffset};
use thinkrail_ui::metrics::space;

use crate::{appearance::AppearanceState, strings::settings_appearance as s};

/// Where an open font picker writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerTarget {
    Family(FontRole),
    Fallback(FontRole),
}

pub struct AppearancePage {
    /// The fixed theme list or the system pair disclosure.
    themes_open: bool,
    advanced_open: bool,
    picker: Option<PickerTarget>,
    search: Entity<InputState>,
    weight: Entity<SliderState>,
    /// The folder whose path was just copied.
    copied: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl AppearancePage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(s::PICKER_SEARCH));
        let offset = cx
            .try_global::<AppearanceState>()
            .map_or(FontWeightOffset::DEFAULT, |state| {
                state.prefs.font_weight_offset
            });
        let weight = cx.new(|_| {
            SliderState::new()
                .min(f32::from(FontWeightOffset::MIN))
                .max(f32::from(FontWeightOffset::MAX))
                .step(f32::from(FontWeightOffset::STEP))
                .default_value(f32::from(offset))
        });
        let subscriptions = vec![
            cx.observe_global::<AppearanceState>(|_, cx| cx.notify()),
            cx.subscribe(
                &search,
                |_, _, _: &gpui_kit::component::input::InputEvent, cx| cx.notify(),
            ),
            // The original commits on release; dragging previews nothing it cannot store.
            cx.subscribe(&weight, |_, _, event: &SliderEvent, cx| {
                if let SliderEvent::Release(value) = event {
                    let offset = value.end().round() as i16;
                    AppearanceState::edit(s::FONTS_CHANGE_FAILED, cx, |prefs| {
                        prefs.font_weight_offset = offset;
                    });
                }
            }),
        ];
        Self {
            themes_open: false,
            advanced_open: false,
            picker: None,
            search,
            weight,
            copied: None,
            _subscriptions: subscriptions,
        }
    }

    /// Replaces the preferences with a selection result (`None`: nothing changes).
    fn choose(next: Option<AppearancePrefs>, cx: &mut Context<Self>) {
        if let Some(next) = next {
            AppearanceState::save(next, s::CHANGE_FAILED, cx);
        }
    }

    /// Keeps the slider on the stored value after a reset or a failed write.
    fn sync_weight(&self, offset: i16, window: &mut Window, cx: &mut Context<Self>) {
        let value = f32::from(offset);
        if (self.weight.read(cx).value().end() - value).abs() > f32::EPSILON {
            self.weight
                .update(cx, |slider, cx| slider.set_value(value, window, cx));
        }
    }
}

impl Render for AppearancePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(offset) = cx
            .try_global::<AppearanceState>()
            .map(|state| state.prefs.font_weight_offset)
        else {
            return v_flex().id("settings-appearance");
        };
        self.sync_weight(offset, window, cx);
        v_flex()
            .id("settings-appearance")
            .gap(space::S24)
            .child(self.theme_group(cx))
            .child(self.fonts_group(window, cx))
            .child(self.custom_fonts(cx))
            .child(self.custom_themes(cx))
    }
}

#[cfg(test)]
mod tests;
