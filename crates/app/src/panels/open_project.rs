//! 打开项目 / 新建项目, shared by the projects rail "+" and the welcome page (original ThinkRail
//! `useOpenProject`): a system folder picker; a folder outside any Git repository is offered
//! `git init`; 新建项目 makes `<位置>/<名称>` with a first commit.

use std::path::PathBuf;

use gpui_kit::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, PathPromptOptions,
    Render, Styled as _, Subscription, Window,
    base::{h_flex, v_flex},
    component::{
        Disableable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
        input::{Input, InputEvent, InputState},
        switch::Switch,
    },
    px,
};
use thinkrail_core::Readiness;
use thinkrail_ui::{
    ThinkrailTheme,
    components::label,
    metrics::{space, text},
};

use crate::{
    state::{AppState, run},
    strings::projects as s,
};

/// The folder picker, then open (or offer `git init`).
pub fn open(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(s::OPEN.into()),
    });
    let workspace = state.read(cx).services.workspace.clone();
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picked.await else {
            return;
        };
        let Some(path) = paths.into_iter().next() else {
            return;
        };
        let probe = path.clone();
        let repo = cx
            .background_executor()
            .spawn(async move { workspace.is_repository(&probe) })
            .await;
        let _ = handle.update(cx, |_, window, cx| {
            if repo {
                state.update(cx, |state, cx| state.open_project(path, cx));
            } else {
                confirm_init(state, path, window, cx);
            }
        });
    })
    .detach();
}

fn confirm_init(state: Entity<AppState>, path: PathBuf, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (state, path) = (state.clone(), path.clone());
        alert
            .title(s::INIT_TITLE)
            .description(s::init_body(&path))
            .ok_text(s::INIT)
            .show_cancel(true)
            .on_ok(move |_, _, cx| {
                let path = path.clone();
                state.update(cx, |state, cx| {
                    let workspace = state.services.workspace.clone();
                    run(
                        cx,
                        move || workspace.init_project(&path),
                        |state: &mut AppState, result, cx| match result {
                            Ok(project) => state.opened(project, cx),
                            Err(error) => state.report(&error, cx),
                        },
                    );
                });
                true
            })
    });
}

/// Default location for new projects: `~/thinkrail` (original ThinkRail).
fn default_parent() -> String {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("thinkrail").display().to_string())
        .unwrap_or_default()
}

struct CreateForm {
    state: Entity<AppState>,
    name: Entity<InputState>,
    parent: Entity<InputState>,
    creating: bool,
    /// 让 Agent 起草规格 (on by default, like the original); applies only when `readiness` is ready.
    setup: bool,
    /// The default Agent's state, checked when the dialog opens (07 §5.2); `None` while checking.
    readiness: Option<Readiness>,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl CreateForm {
    fn target(&self, cx: &App) -> Option<PathBuf> {
        let name = self.name.read(cx).value().trim().to_owned();
        let parent = self.parent.read(cx).value().trim().to_owned();
        (!name.is_empty() && !parent.is_empty()).then(|| PathBuf::from(parent).join(name))
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.parent.update(cx, |input, cx| {
                    input.set_value(path.display().to_string(), window, cx)
                });
            });
        })
        .detach();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_owned();
        let parent = PathBuf::from(self.parent.read(cx).value().trim());
        let setup = self.setup && matches!(self.readiness, Some(Readiness::Ready(_)));
        if self.creating {
            return;
        }
        // An empty field is said out loud and focused, never a silent no-op.
        let missing = if name.is_empty() {
            Some((s::ERR_NAME, &self.name))
        } else if parent.as_os_str().is_empty() {
            Some((s::ERR_LOCATION, &self.parent))
        } else {
            None
        };
        if let Some((message, field)) = missing {
            self.error = Some(message.to_owned());
            field.update(cx, |input, cx| input.focus(window, cx));
            cx.notify();
            return;
        }
        self.creating = true;
        self.error = None;
        let workspace = self.state.read(cx).services.workspace.clone();
        let handle = window.window_handle();
        run(
            cx,
            move || workspace.create_project(&parent, &name),
            move |this: &mut CreateForm, result, cx| {
                this.creating = false;
                match result {
                    Ok(project) => {
                        this.state.update(cx, |state, cx| {
                            if setup {
                                state.setup_project = Some(project.id.clone())
                            }
                            state.opened(project, cx)
                        });
                        let _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
}

impl Render for CreateForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let preview = self.target(cx).map_or_else(
            || s::CREATE_PATH_HINT.to_owned(),
            |p| p.display().to_string(),
        );
        let field =
            |title: &'static str| label(text::TITLE_COMPACT, title).text_color(r.text_muted);
        v_flex()
            .gap(space::S12)
            .child(label(text::METADATA, s::CREATE_DESC).text_color(r.text_subtle))
            .child(
                v_flex()
                    .gap(space::S4)
                    .child(field(s::CREATE_NAME))
                    .child(Input::new(&self.name)),
            )
            .child(
                v_flex()
                    .gap(space::S4)
                    .child(field(s::CREATE_LOCATION))
                    .child(
                        h_flex()
                            .gap(space::S8)
                            .child(Input::new(&self.parent).flex_1())
                            .child(
                                Button::new("create-browse")
                                    .outline()
                                    .label(s::BROWSE)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.browse(window, cx)),
                                    ),
                            ),
                    ),
            )
            .child(label(text::CODE_SMALL, preview).text_color(r.text_subtle))
            .child(
                match self.readiness.filter(|r| !matches!(r, Readiness::Ready(_))) {
                    Some(readiness) => v_flex()
                        .children(super::agent_guide::row(
                            "create-guide",
                            self.state.clone(),
                            readiness,
                            cx,
                        ))
                        .into_any_element(),
                    None => h_flex()
                        .gap(space::S12)
                        .items_center()
                        .child(
                            v_flex()
                                .flex_1()
                                .child(label(text::TITLE_COMPACT, s::SETUP_TITLE))
                                .child(
                                    label(text::METADATA, s::SETUP_DETAIL)
                                        .text_color(r.text_subtle),
                                ),
                        )
                        .child(
                            Switch::new("create-setup")
                                .checked(self.setup)
                                .disabled(self.readiness.is_none())
                                .accessibility_label(s::SETUP_TITLE)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.setup = *checked;
                                    cx.notify()
                                })),
                        )
                        .into_any_element(),
                },
            )
            .children(
                self.error
                    .clone()
                    .map(|e| label(text::METADATA, e).text_color(r.feedback_error)),
            )
    }
}

/// 新建项目 dialog.
pub fn create(state: Entity<AppState>, window: &mut Window, cx: &mut App) {
    let form = cx.new(|cx| {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(s::CREATE_NAME_PLACEHOLDER));
        let parent = cx.new(|cx| InputState::new(window, cx).default_value(default_parent()));
        let subscriptions = vec![
            cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify()
                }
            }),
            cx.subscribe(&parent, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify()
                }
            }),
        ];
        let services = state.read(cx).services.clone();
        run(
            cx,
            move || services.readiness().map(|(readiness, _)| readiness),
            |this: &mut CreateForm, result, cx| {
                match result {
                    Ok(readiness) => this.readiness = Some(readiness),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            },
        );
        CreateForm {
            state,
            name,
            parent,
            creating: false,
            setup: true,
            readiness: None,
            error: None,
            _subscriptions: subscriptions,
        }
    });
    let focus = form.read(cx).name.clone();
    focus.update(cx, |input, cx| input.focus(window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let (submit, enter, busy) = (form.clone(), form.clone(), form.read(cx).creating);
        dialog
            .title(s::CREATE_TITLE)
            .w(px(520.))
            .child(form.clone())
            .on_ok(move |_, window, cx| {
                enter.update(cx, |f, cx| f.submit(window, cx));
                false
            })
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("create-cancel")
                            .outline()
                            .label(s::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("create-submit")
                            .primary()
                            .label(if busy { s::CREATING } else { s::CREATE })
                            .on_click(move |_, window, cx| {
                                submit.update(cx, |f, cx| f.submit(window, cx))
                            }),
                    ),
            )
    });
}
