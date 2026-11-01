//! Welcome (original ThinkRail `Welcome`, no-projects state): the wordmark and two cards,
//! 新建项目 (primary) and 打开项目.

use gpui_kit::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, StatefulInteractiveElement as _,
    Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::Icon,
    px,
};
use thinkrail_ui::{ThinkrailTheme, components::label, metrics::{radius, space, text}};

use super::open_project;
use crate::{state::AppState, strings::projects as s};

pub struct Welcome {
    state: Entity<AppState>,
}

impl Welcome {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }
}

impl Render for Welcome {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let card = |id: &'static str, icon: IconName, title: &'static str, body: &'static str, primary: bool| {
            v_flex().id(id).w(px(244.)).h(px(166.)).p(space::S16).gap(space::S4).justify_end().rounded(radius::LG).border_1()
                .cursor_pointer()
                .bg(if primary { r.primary_subtle } else { r.container_elevated_bg })
                .border_color(if primary { r.primary_muted } else { r.border_muted })
                .hover(|s| s.border_color(r.primary))
                .child(Icon::new(icon).size(px(22.)).text_color(if primary { r.primary } else { r.text_muted }).mb_auto())
                .child(label(text::TITLE_CARD, title))
                .child(label(text::METADATA, body).text_color(r.text_muted))
        };
        let (create, open) = (self.state.clone(), self.state.clone());
        v_flex().size_full().items_center().justify_center().gap(space::S24)
            .child(label(text::BRAND_HERO, s::BRAND).font_family(thinkrail_ui::fonts::BRAND).text_color(r.primary))
            .child(h_flex().gap(space::S12)
                .child(card("welcome-create", IconName::Sparkles, s::CREATE_TITLE, s::CREATE_SUBTITLE, true)
                    .on_click(move |_, window, cx| open_project::create(create.clone(), window, cx)))
                .child(card("welcome-open", IconName::FolderOpen, s::OPEN, s::OPEN_SUBTITLE, false)
                    .on_click(move |_, window, cx| open_project::open(open.clone(), window, cx))))
    }
}
