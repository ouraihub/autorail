//! 编码搭档 (agent-settings 04 §7.10, after Cherry Studio `CodeCliPage`): engines on the left with
//! their install state; on the right install / 设为默认 and the one provider row the engine uses.

use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, Icon, Sizable as _,
        button::Button,
        input::{Input, InputEvent, InputState},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{
    BackendKind, EngineInstall, EngineProvider, EngineSource, Error, Launch, ModelPrefs, Provider,
};
use thinkrail_ui::{
    ThinkrailTheme,
    components::label,
    metrics::{radius, space, text},
};

use super::providers::avatar;
use crate::{
    state::{AppEvent, AppState, run},
    strings::settings_coding as s,
};

pub struct CodingPage {
    state: Entity<AppState>,
    selected: BackendKind,
    /// `None` until the first status check returns.
    installs: [Option<Option<EngineInstall>>; 3],
    installing: Option<BackendKind>,
    install_error: Option<(BackendKind, String)>,
    providers: Vec<Provider>,
    prefs: ModelPrefs,
    search: Entity<InputState>,
    /// Asks the settings shell to show 模型服务.
    pub(super) open_providers: bool,
    _subscriptions: Vec<Subscription>,
}

fn index(backend: BackendKind) -> usize {
    BackendKind::ALL
        .iter()
        .position(|b| *b == backend)
        .unwrap_or(0)
}

impl CodingPage {
    pub fn new(
        state: Entity<AppState>,
        selected: BackendKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(s::SEARCH));
        let subscriptions = vec![
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify()
                }
            }),
            cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
                if matches!(event, AppEvent::Routing) {
                    this.reload(cx)
                }
            }),
        ];
        let mut page = Self {
            state,
            selected,
            installs: [None, None, None],
            installing: None,
            install_error: None,
            providers: Vec::new(),
            prefs: ModelPrefs::default(),
            search,
            open_providers: false,
            _subscriptions: subscriptions,
        };
        page.reload(cx);
        for backend in BackendKind::ALL {
            page.check(backend, cx);
        }
        page
    }

    fn report(&mut self, error: &Error, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.report(error, cx));
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let services = &self.state.read(cx).services;
        let (providers, prefs) = (services.providers.clone(), services.prefs.clone());
        run(
            cx,
            move || Ok((providers.providers()?, prefs.prefs()?)),
            |this, result, cx| {
                match result {
                    Ok((providers, prefs)) => {
                        this.providers = providers;
                        this.prefs = prefs;
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn check(&mut self, backend: BackendKind, cx: &mut Context<Self>) {
        let engines = self.state.read(cx).services.engines.clone();
        run(
            cx,
            move || engines.status(backend),
            move |this, result, cx| {
                match result {
                    Ok(found) => this.installs[index(backend)] = Some(found),
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }

    fn install(&mut self, backend: BackendKind, cx: &mut Context<Self>) {
        if self.installing.is_some() {
            return;
        }
        self.installing = Some(backend);
        self.install_error = None;
        let engines = self.state.read(cx).services.engines.clone();
        run(
            cx,
            move || engines.install(backend),
            move |this, result, cx| {
                this.installing = None;
                match result {
                    Ok(found) => this.installs[index(backend)] = Some(Some(found)),
                    Err(error) => this.install_error = Some((backend, error.to_string())),
                }
                cx.notify();
            },
        );
    }

    fn choose(&mut self, backend: BackendKind, choice: EngineProvider, cx: &mut Context<Self>) {
        let prefs = self.state.read(cx).services.prefs.clone();
        run(
            cx,
            move || prefs.set_engine_provider(backend, choice),
            |this, result, cx| {
                match result {
                    Ok(prefs) => {
                        this.prefs = prefs;
                        this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    }
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
}

impl CodingPage {
    fn is_default(&self, backend: BackendKind) -> bool {
        self.prefs.default_agent == Some(backend)
    }

    /// New chats run this engine (agent-settings 07 §5.4); existing chats keep theirs.
    fn set_default(&mut self, backend: BackendKind, cx: &mut Context<Self>) {
        let prefs = self.state.read(cx).services.prefs.clone();
        run(
            cx,
            move || prefs.set_default_agent(Some(backend)),
            |this, result, cx| {
                match result {
                    Ok(prefs) => this.prefs = prefs,
                    Err(error) => this.report(&error, cx),
                }
                cx.notify();
            },
        );
    }
}

impl Render for CodingPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let rows = BackendKind::ALL.map(|backend| {
            let current = backend == self.selected;
            let status = match (
                &self.installs[index(backend)],
                self.installing == Some(backend),
            ) {
                (_, true) => Some(s::INSTALLING),
                (Some(None), false) => Some(s::NOT_INSTALLED),
                _ if self.is_default(backend) => Some(s::IS_DEFAULT),
                _ => None,
            };
            h_flex()
                .id(("coding-engine", index(backend)))
                .h(px(40.))
                .px(space::S8)
                .gap(space::S8)
                .rounded(radius::MD)
                .cursor_pointer()
                .when(current, |d| d.bg(r.control_bg_selected))
                .when(!current, |d| d.hover(|s| s.bg(r.control_bg_hovered)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = backend;
                    cx.notify();
                }))
                .child(avatar(s::name(backend), cx))
                .child(label(text::UI, s::name(backend)).flex_1())
                .children(status.map(|t| label(text::METADATA, t).text_color(r.text_subtle)))
        });
        h_flex()
            .w_full()
            .items_start()
            .gap(space::S16)
            .child(
                v_flex()
                    .w(px(208.))
                    .flex_none()
                    .gap(space::S4)
                    .children(rows),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(space::S16)
                    .child(self.render_header(cx))
                    .child(self.render_providers(cx)),
            )
    }
}

impl CodingPage {
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let backend = self.selected;
        let install = self.installs[index(backend)].clone();
        let installing = self.installing == Some(backend);
        let failed = self
            .install_error
            .as_ref()
            .filter(|(b, _)| *b == backend)
            .map(|(_, e)| e.clone());
        let badge = install
            .as_ref()
            .and_then(Option::as_ref)
            .map(|i| match i.source {
                EngineSource::Managed => s::MANAGED,
                EngineSource::System => s::SYSTEM,
            });
        let detail = install.as_ref().and_then(Option::as_ref).map(|i| {
            let Launch::External { executable, args } = &i.launch;
            let path = args
                .first()
                .map_or_else(|| executable.display().to_string(), Clone::clone);
            i.version
                .as_deref()
                .map_or(path.clone(), |v| format!("{} · {path}", s::version(v)))
        });
        let action = match &install {
            None => None,
            Some(None) => Some(
                Button::new("coding-install")
                    .outline()
                    .icon(IconName::ArrowDown)
                    .label(if installing {
                        s::INSTALLING
                    } else if failed.is_some() {
                        s::RETRY
                    } else {
                        s::INSTALL
                    })
                    .disabled(installing || self.installing.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| this.install(backend, cx))),
            ),
            Some(Some(_)) if self.is_default(backend) => None,
            Some(Some(_)) => Some(
                Button::new("coding-default")
                    .outline()
                    .icon(IconName::Star)
                    .label(s::SET_DEFAULT)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_default(backend, cx))),
            ),
        };
        v_flex()
            .p(space::S16)
            .gap(space::S8)
            .rounded(radius::LG)
            .border_1()
            .border_color(r.border_muted)
            .child(
                h_flex()
                    .gap(space::S12)
                    .child(avatar(s::name(backend), cx))
                    .child(label(text::TITLE_SECTION, s::name(backend)))
                    .children(badge.map(|b| {
                        label(text::METADATA, b)
                            .px(space::S6)
                            .rounded(radius::SM)
                            .bg(r.control_bg)
                            .text_color(r.text_muted)
                    }))
                    .when(self.is_default(backend), |d| {
                        d.child(
                            label(text::METADATA, s::IS_DEFAULT)
                                .px(space::S6)
                                .rounded(radius::SM)
                                .bg(r.primary_subtle)
                                .text_color(r.primary),
                        )
                    })
                    .child(div().flex_1())
                    .children(action),
            )
            .children(detail.map(|d| {
                label(text::CODE_SMALL, d)
                    .text_color(r.text_subtle)
                    .truncate()
            }))
            .when(installing, |d| {
                d.child(label(text::METADATA, s::INSTALLING_HINT).text_color(r.text_subtle))
            })
            .children(failed.map(|e| label(text::METADATA, e).text_color(r.feedback_error)))
    }

    fn render_providers(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let backend = self.selected;
        let chosen = self.prefs.engine_provider(backend).cloned();
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut rows: Vec<(EngineProvider, String, Option<&'static str>)> = vec![
            (
                EngineProvider::Gateway,
                s::GATEWAY.into(),
                Some(s::GATEWAY_DESC),
            ),
            (
                EngineProvider::Official,
                s::official(backend),
                Some(s::OFFICIAL_DESC),
            ),
        ];
        rows.extend(self.providers.iter().filter(|p| p.input.enabled).map(|p| {
            (
                EngineProvider::Provider(p.id.clone()),
                p.input.name.clone(),
                None,
            )
        }));
        rows.retain(|(_, name, _)| query.is_empty() || name.to_lowercase().contains(&query));
        let empty = rows.is_empty();
        let list = rows
            .into_iter()
            .enumerate()
            .map(|(i, (choice, name, desc))| {
                let on = chosen.as_ref() == Some(&choice);
                let pick = choice.clone();
                h_flex()
                    .id(("coding-provider", i))
                    .min_h(px(52.))
                    .px(space::S12)
                    .py(space::S8)
                    .gap(space::S12)
                    .rounded(radius::LG)
                    .border_1()
                    .border_color(if on {
                        r.control_border_active
                    } else {
                        r.border_muted
                    })
                    .child(match &choice {
                        EngineProvider::Gateway => div()
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::Plug).small())
                            .into_any_element(),
                        _ => avatar(&name, cx).into_any_element(),
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(label(text::UI, name))
                            .children(
                                desc.map(|d| label(text::METADATA, d).text_color(r.text_subtle)),
                            ),
                    )
                    .child(if on {
                        label(text::METADATA, s::ENABLED)
                            .text_color(r.feedback_success)
                            .into_any_element()
                    } else {
                        Button::new(("coding-enable", i))
                            .outline()
                            .small()
                            .label(s::ENABLE)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.choose(backend, pick.clone(), cx)
                            }))
                            .into_any_element()
                    })
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap(space::S8)
            .child(
                h_flex()
                    .gap(space::S8)
                    .child(label(text::TITLE_SECTION, s::PROVIDERS))
                    .child(div().flex_1())
                    .child(
                        Input::new(&self.search)
                            .prefix(Icon::new(IconName::Search).small())
                            .w(px(240.)),
                    ),
            )
            .when(chosen.is_none(), |d| {
                d.child(label(text::METADATA, s::ALL_PROVIDERS).text_color(r.text_subtle))
            })
            .children(list)
            .when(empty, |d| {
                d.child(label(text::METADATA, s::NO_MATCHES).text_color(r.text_subtle))
            })
            .child(
                h_flex()
                    .id("coding-add-provider")
                    .h(px(36.))
                    .justify_center()
                    .gap(space::S6)
                    .rounded(radius::LG)
                    .border_1()
                    .border_dashed()
                    .border_color(r.border_muted)
                    .cursor_pointer()
                    .hover(|s| s.bg(r.control_bg_hovered))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_providers = true;
                        cx.notify();
                    }))
                    .child(label(text::METADATA, s::ADD_PROVIDER).text_color(r.text_muted))
                    .child(Icon::new(IconName::ExternalLink).small()),
            )
    }
}
