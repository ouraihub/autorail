mod coding;
mod models;
mod providers;
mod mcp;

use gpui_kit::{App, AppContext as _, Context, Entity, IntoElement, InteractiveElement as _, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window, base::{h_flex, v_flex}, component::{Icon, WindowExt as _, scroll::ScrollableElement as _}, assets::IconName, prelude::FluentBuilder as _, px};
use thinkrail_core::BackendKind;
use thinkrail_ui::{ThinkrailTheme, components::label, metrics::{radius, size, space, text}};
use crate::{state::AppState, strings::{settings as shell, settings_coding as s_coding, settings_mcp as sm, settings_models as s_models, settings_providers as s_providers}};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page { Models, Providers, Coding, Mcp }

pub struct SettingsDialog;

struct SettingsView {
    page: Page,
    models: Entity<models::ModelsPage>,
    providers: Entity<providers::ProvidersPage>,
    coding: Entity<coding::CodingPage>,
    mcp: Entity<mcp::McpPage>,
    _coding: gpui_kit::Subscription,
}

impl SettingsDialog {
    pub fn open(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
        Self::show(state, None, window, cx);
    }

    /// Opens 编码搭档 with `backend` selected (the 新建项目 / chat guidance, agent-settings 07 §5.1).
    pub fn open_coding(state: Entity<AppState>, backend: BackendKind, window: &mut Window, cx: &mut App) {
        Self::show(state, Some(backend), window, cx);
    }

    fn show(state: Entity<AppState>, coding_at: Option<BackendKind>, window: &mut Window, cx: &mut App) {
        let models = cx.new(|cx| models::ModelsPage::new(state.clone(), window, cx));
        let providers = cx.new(|cx| providers::ProvidersPage::new(state.clone(), window, cx));
        let coding = cx.new(|cx| coding::CodingPage::new(state.clone(), coding_at.unwrap_or(BackendKind::Pi), window, cx));
        let mcp = cx.new(|cx| mcp::McpPage::new(state.clone(), window, cx));
        let view = cx.new(|cx| {
            // 编码搭档's footer link: jump to 模型服务.
            let watch = cx.observe(&coding, |this: &mut SettingsView, coding, cx| {
                if coding.update(cx, |page, _| std::mem::take(&mut page.open_providers)) {
                    this.page = Page::Providers;
                    cx.notify();
                }
            });
            let page = if coding_at.is_some() { Page::Coding } else { Page::Models };
            SettingsView { page, models, providers, coding, mcp, _coding: watch }
        });
        // Enter inside a field must not close settings: the dialog has no OK action.
        window.open_dialog(cx, move |dialog, _, _| dialog.title(shell::TITLE).w(px(1040.)).on_ok(|_, _, _| false).child(view.clone()));
    }
}

impl SettingsView {
    fn nav(&self, page: Page, icon: IconName, title: &'static str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.page == page;
        let r = ThinkrailTheme::global(cx).roles;
        let id = format!("settings-nav-{title}");
        v_flex().id(id).h(px(28.)).px(space::S8).rounded(radius::SM).justify_center()
            .when(active, |d| d.bg(r.primary_subtle))
            .when(!active, |d| d.hover(|s| s.bg(r.control_bg_hovered)).cursor_pointer())
            .child(h_flex().gap(space::S8)
                .child(Icon::new(icon).size(size::ICON_SMALL))
                .child(label(text::UI, title).text_color(if active { r.primary } else { r.text_default })))
            .when(!active, |d| d.on_click(cx.listener(move |this, _, window, cx| {
                if this.page == Page::Providers {
                    this.providers.update(cx, |view, cx| view.clear_sensitive(window, cx));
                }
                this.page = page;
                cx.notify();
            })))
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let content = match self.page {
            Page::Models => self.models.clone().into_any_element(),
            Page::Providers => self.providers.clone().into_any_element(),
            Page::Coding => self.coding.clone().into_any_element(),
            Page::Mcp => self.mcp.clone().into_any_element(),
        };
        h_flex().id("settings-shell").items_start().h(window.viewport_size().height * 0.8 - px(72.)).min_h(px(0.)).bg(r.container_content_bg)
            .child(v_flex().w(px(192.)).h_full().flex_none().gap(space::S4).p(space::S12).bg(r.container_elevated_bg).border_r_1().border_color(r.border_muted)
                .child(label(text::LABEL_PILL, "AGENT").h(px(28.)).text_color(r.text_subtle))
                .child(self.nav(Page::Models, IconName::Brain, s_models::TITLE, cx))
                .child(self.nav(Page::Providers, IconName::KeyRound, s_providers::TITLE, cx))
                .child(self.nav(Page::Coding, IconName::SquareTerminal, s_coding::TITLE, cx))
                .child(self.nav(Page::Mcp, IconName::Plug, sm::TITLE, cx)))
            .child(v_flex().id("settings-content").flex_1().h_full().min_w(px(0.)).min_h(px(0.)).overflow_y_scrollbar().p(space::S16).child(content))
    }
}
