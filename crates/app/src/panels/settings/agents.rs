//! Settings dialog: the Agent profile catalog (04 §8). Edits save a new revision; existing chats
//! keep the revision they were created with.

use std::path::{Path, PathBuf};

use gpui_kit::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{
        button::{Button, ButtonVariants as _},
        input::{Input, InputState},
    },
    div,
    prelude::FluentBuilder as _,
    px,
};
use thinkrail_core::{BackendKind, EnvRef, EnvSource, Launch, Profile, ProfileCatalog, ProfileId, ProfileInput};
use thinkrail_ui::{
    ThinkrailTheme,
    components::{IconButton, Segment, label},
    metrics::{radius, size, space, text},
};

use crate::{
    state::{AppState, run},
    strings::settings as s,
};

const BACKENDS: [BackendKind; 3] = [BackendKind::Pi, BackendKind::Omp, BackendKind::Dsh];


/// The raw form fields, before validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Form {
    pub backend: BackendKind,
    pub label: String,
    pub executable: String,
    pub args: String,
    pub config_root: String,
    pub env: String,
    pub enabled: bool,
}

/// Validates the form into a profile; the message names the first bad field.
pub(crate) fn to_input(form: &Form) -> Result<ProfileInput, &'static str> {
    let label = form.label.trim();
    if label.is_empty() {
        return Err(s::ERR_LABEL);
    }
    let executable = PathBuf::from(form.executable.trim());
    if !executable.is_absolute() {
        return Err(s::ERR_EXECUTABLE);
    }
    let args = match form.args.trim() {
        "" => Vec::new(),
        raw => serde_json::from_str::<Vec<String>>(raw).map_err(|_| s::ERR_ARGS)?,
    };
    let config_root = match form.config_root.trim() {
        "" => None,
        raw if Path::new(raw).is_absolute() => Some(PathBuf::from(raw)),
        _ => return Err(s::ERR_CONFIG_ROOT),
    };
    let mut env_refs = Vec::new();
    for line in form.env.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (name, reference) = line.split_once('=').ok_or(s::ERR_ENV)?;
        let (name, reference) = (name.trim(), reference.trim());
        if name.is_empty() || reference.is_empty() {
            return Err(s::ERR_ENV);
        }
        env_refs.push(EnvRef { name: name.into(), source: EnvSource::Launch, reference: reference.into() });
    }
    Ok(ProfileInput {
        backend_id: form.backend,
        label: label.into(),
        enabled: form.enabled,
        config_root,
        launch: Launch::External { executable, args },
        env_refs,
    })
}

fn backend_index(backend: BackendKind) -> usize {
    BACKENDS.iter().position(|b| *b == backend).unwrap_or(0)
}

fn example(backend: BackendKind) -> &'static str {
    match backend {
        BackendKind::Pi => s::EXAMPLE_PI,
        BackendKind::Omp => s::EXAMPLE_OMP,
        BackendKind::Dsh => s::EXAMPLE_DSH,
    }
}

struct Editing {
    /// `None` creates a new profile.
    id: Option<ProfileId>,
    revision: u64,
}

struct Fields {
    label: Entity<InputState>,
    executable: Entity<InputState>,
    args: Entity<InputState>,
    config_root: Entity<InputState>,
    env: Entity<InputState>,
}

pub(crate) struct SettingsView {
    state: Entity<AppState>,
    catalog: ProfileCatalog,
    editing: Option<Editing>,
    backend: BackendKind,
    enabled: bool,
    fields: Fields,
    form_error: Option<&'static str>,
    confirm_delete: Option<ProfileId>,
}

impl SettingsView {
    pub(crate) fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let fields = Fields {
            label: input(s::LABEL, window, cx),
            executable: input(s::EXECUTABLE, window, cx),
            args: input(s::ARGS, window, cx),
            config_root: input(s::CONFIG_ROOT, window, cx),
            env: input(s::ENV_REFS, window, cx),
        };
        let mut view = Self {
            state,
            catalog: ProfileCatalog::default(),
            editing: None,
            backend: BackendKind::Pi,
            enabled: true,
            fields,
            form_error: None,
            confirm_delete: None,
        };
        view.reload(cx);
        view
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let profiles = self.state.read(cx).services.profiles.clone();
        run(cx, move || profiles.catalog(), |this, result, cx| match result {
            Ok(catalog) => {
                this.catalog = catalog;
                cx.notify();
            }
            Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
        });
    }

    fn start_edit(&mut self, profile: Option<&Profile>, window: &mut Window, cx: &mut Context<Self>) {
        let (label, executable, args, root, env) = match profile {
            Some(p) => {
                let Launch::External { executable, args } = &p.input.launch;
                let env = p.input.env_refs.iter().map(|e| format!("{}={}", e.name, e.reference)).collect::<Vec<_>>().join("\n");
                (
                    p.input.label.clone(),
                    executable.display().to_string(),
                    serde_json::to_string(args).unwrap_or_default(),
                    p.input.config_root.as_ref().map(|r| r.display().to_string()).unwrap_or_default(),
                    env,
                )
            }
            None => (String::new(), String::new(), "[]".to_owned(), String::new(), String::new()),
        };
        self.backend = profile.map_or(BackendKind::Pi, |p| p.input.backend_id);
        self.enabled = profile.is_none_or(|p| p.input.enabled);
        self.editing = Some(Editing { id: profile.map(|p| p.id.clone()), revision: profile.map_or(0, |p| p.config_revision) });
        self.form_error = None;
        let set = |field: &Entity<InputState>, value: String, window: &mut Window, cx: &mut Context<Self>| {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        };
        set(&self.fields.label.clone(), label, window, cx);
        set(&self.fields.executable.clone(), executable, window, cx);
        set(&self.fields.args.clone(), args, window, cx);
        set(&self.fields.config_root.clone(), root, window, cx);
        set(&self.fields.env.clone(), env, window, cx);
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = &self.editing else { return };
        let value = |field: &Entity<InputState>, cx: &App| field.read(cx).value().to_string();
        let form = Form {
            backend: self.backend,
            label: value(&self.fields.label, cx),
            executable: value(&self.fields.executable, cx),
            args: value(&self.fields.args, cx),
            config_root: value(&self.fields.config_root, cx),
            env: value(&self.fields.env, cx),
            enabled: self.enabled,
        };
        let input = match to_input(&form) {
            Ok(input) => input,
            Err(message) => {
                self.form_error = Some(message);
                cx.notify();
                return;
            }
        };
        let (id, revision) = (editing.id.clone(), editing.revision);
        let profiles = self.state.read(cx).services.profiles.clone();
        run(
            cx,
            move || match id {
                Some(id) => profiles.update(&id, revision, input).map(drop),
                None => profiles.create(input).map(drop),
            },
            |this, result, cx| {
                match result {
                    Ok(()) => {
                        this.editing = None;
                        this.form_error = None;
                    }
                    Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
                }
                this.reload(cx);
            },
        );
    }

    fn set_default(&mut self, id: ProfileId, cx: &mut Context<Self>) {
        let profiles = self.state.read(cx).services.profiles.clone();
        run(cx, move || profiles.set_default(Some(&id)), |this, result, cx| match result {
            Ok(catalog) => {
                this.catalog = catalog;
                cx.notify();
            }
            Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
        });
    }

    fn delete(&mut self, id: ProfileId, revision: u64, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        let profiles = self.state.read(cx).services.profiles.clone();
        run(cx, move || profiles.delete(&id, revision), |this, result, cx| {
            if let Err(error) = result {
                this.state.update(cx, |state, cx| state.report(&error, cx));
            }
            this.reload(cx);
        });
    }

    fn row(&self, profile: &Profile, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let is_default = self.catalog.default_profile_id.as_ref() == Some(&profile.id);
        let confirming = self.confirm_delete.as_ref() == Some(&profile.id);
        let root = profile.input.config_root.as_ref().map_or_else(|| s::ENGINE_DEFAULT_DIR.to_owned(), |p| p.display().to_string());
        let (edit_profile, default_id, delete_id, revision) =
            (profile.clone(), profile.id.clone(), profile.id.clone(), profile.config_revision);
        let backend = profile.input.backend_id;
        h_flex()
            .id(format!("profile-{}", profile.id))
            .gap(space::S8)
            .px(space::S8)
            .py(space::S6)
            .rounded(radius::SM)
            .border_1()
            .border_color(r.border_muted)
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .gap(space::S2)
                    .child(
                        h_flex()
                            .gap(space::S6)
                            .child(label(text::LABEL_PILL, backend.label()).text_color(r.primary))
                            .when(backend == BackendKind::Dsh, |d| {
                                d.child(label(text::CAPTION, s::PRERELEASE).px(space::S4).rounded(radius::XS).bg(r.feedback_warning_subtle).text_color(r.feedback_warning))
                            })
                            .child(label(text::TITLE_SECTION, profile.input.label.clone()).truncate())
                            .when(is_default, |d| d.child(label(text::CAPTION, s::DEFAULT).text_color(r.feedback_success))),
                    )
                    .child(label(text::METADATA, root).truncate().text_color(r.text_subtle))
                    .child(
                        label(
                            text::METADATA,
                            format!(
                                "{} {} · {}",
                                s::REVISION,
                                profile.config_revision,
                                if profile.input.enabled { s::ENABLED } else { s::DISABLED }
                            ),
                        )
                        .text_color(r.text_muted),
                    ),
            )
            .when(!is_default, |d| {
                d.child(IconButton::new(format!("default-{}", profile.id), IconName::Star).tooltip(s::SET_DEFAULT).on_click(
                    cx.listener(move |this, _, _, cx| this.set_default(default_id.clone(), cx)),
                ))
            })
            .child(IconButton::new(format!("edit-{}", profile.id), IconName::Pencil).tooltip(s::EDIT).on_click(cx.listener(
                move |this, _, window, cx| this.start_edit(Some(&edit_profile), window, cx),
            )))
            .child(if confirming {
                Button::new(format!("confirm-delete-{}", profile.id))
                    .danger()
                    .label(s::CONFIRM_DELETE)
                    .on_click(cx.listener(move |this, _, _, cx| this.delete(delete_id.clone(), revision, cx)))
                    .into_any_element()
            } else {
                let id = profile.id.clone();
                IconButton::new(format!("delete-{}", profile.id), IconName::Trash)
                    .tooltip(s::DELETE)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirm_delete = Some(id.clone());
                        cx.notify();
                    }))
                    .into_any_element()
            })
    }

    fn form(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let editing_existing = self.editing.as_ref().and_then(|e| e.id.as_ref().map(|_| e.revision));
        let field = |title: &'static str, input: &Entity<InputState>| {
            v_flex().gap(space::S2).child(label(text::TITLE_COMPACT, title).text_color(r.text_muted)).child(Input::new(input))
        };
        v_flex()
            .gap(space::S8)
            .p(space::S12)
            .rounded(radius::MD)
            .bg(r.container_content_bg)
            .child(label(text::TITLE_CARD, if editing_existing.is_some() { s::FORM_EDIT } else { s::FORM_NEW }))
            .when_some(editing_existing, |d, revision| {
                d.child(label(text::METADATA, format!("{} {revision}", s::KEEP_REVISION)).text_color(r.text_subtle))
            })
            .child(
                h_flex()
                    .gap(space::S12)
                    .child(v_flex().gap(space::S2).child(label(text::TITLE_COMPACT, s::BACKEND).text_color(r.text_muted)).child(
                        Segment::new("backend", BACKENDS.map(BackendKind::label), backend_index(self.backend)).on_select(
                            cx.listener(|this, index: &usize, _, cx| {
                                this.backend = BACKENDS[*index];
                                cx.notify();
                            }),
                        ),
                    ))
                    .child(v_flex().gap(space::S2).child(label(text::TITLE_COMPACT, s::STATE).text_color(r.text_muted)).child(
                        Segment::new("enabled", [s::ENABLED, s::DISABLED], usize::from(!self.enabled)).on_select(cx.listener(
                            |this, index: &usize, _, cx| {
                                this.enabled = *index == 0;
                                cx.notify();
                            },
                        )),
                    )),
            )
            .child(field(s::LABEL, &self.fields.label))
            .child(field(s::EXECUTABLE, &self.fields.executable))
            .child(field(s::ARGS, &self.fields.args))
            .child(label(text::METADATA, example(self.backend)).text_color(r.text_subtle))
            .child(label(text::METADATA, s::HOST_FLAGS).text_color(r.text_subtle))
            .child(field(s::CONFIG_ROOT, &self.fields.config_root))
            .child(field(s::ENV_REFS, &self.fields.env))
            .when_some(self.form_error, |d, message| d.child(label(text::METADATA, message).text_color(r.feedback_error)))
            .child(
                h_flex()
                    .gap(space::S8)
                    .justify_end()
                    .child(Button::new("form-cancel").ghost().label(s::CANCEL).on_click(cx.listener(|this, _, _, cx| {
                        this.editing = None;
                        this.form_error = None;
                        cx.notify();
                    })))
                    .child(Button::new("form-save").primary().label(s::SAVE).on_click(cx.listener(|this, _, _, cx| this.save(cx)))),
            )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let rows: Vec<_> = self.catalog.profiles.clone().iter().map(|p| self.row(p, cx).into_any_element()).collect();
        let empty = rows.is_empty();
        let form = self.editing.is_some().then(|| self.form(cx));
        v_flex()
            .id("settings-agents")
            .gap(space::S8)
            .max_h(px(620.))
            .overflow_y_scroll()
            .child(
                h_flex()
                    .h(size::PANEL_HEADER)
                    .child(label(text::TITLE_SECTION, s::AGENTS).flex_1())
                    .when(self.editing.is_none(), |d| {
                        d.child(Button::new("profile-new").primary().icon(IconName::Plus).label(s::NEW).on_click(cx.listener(
                            |this, _, window, cx| this.start_edit(None, window, cx),
                        )))
                    }),
            )
            .children(form)
            .when(empty, |d| d.child(label(text::METADATA, s::EMPTY).text_color(r.text_subtle)))
            .children(rows)
            .child(div().h(space::S4))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form {
            backend: BackendKind::Dsh,
            label: " 工作 ".into(),
            executable: r"C:\Program Files\nodejs\node.exe".into(),
            args: r#"["C:\\dsh\\lib\\bin.js"]"#.into(),
            config_root: String::new(),
            env: "DEEPSEEK_API_KEY = MY_KEY\n\n".into(),
            enabled: true,
        }
    }

    #[test]
    fn valid_form_becomes_a_profile_input() {
        let input = to_input(&form()).unwrap();
        assert_eq!(input.label, "工作");
        assert_eq!(input.config_root, None, "an empty directory uses the engine default");
        let Launch::External { args, .. } = &input.launch;
        assert_eq!(args, &vec![r"C:\dsh\lib\bin.js".to_owned()]);
        assert_eq!(input.env_refs, vec![EnvRef { name: "DEEPSEEK_API_KEY".into(), source: EnvSource::Launch, reference: "MY_KEY".into() }]);
        assert_eq!(to_input(&Form { args: "  ".into(), ..form() }).unwrap().launch, Launch::External {
            executable: r"C:\Program Files\nodejs\node.exe".into(),
            args: Vec::new()
        });
    }

    #[test]
    fn invalid_fields_name_the_first_problem() {
        assert_eq!(to_input(&Form { label: " ".into(), ..form() }), Err(s::ERR_LABEL));
        assert_eq!(to_input(&Form { executable: "node.exe".into(), ..form() }), Err(s::ERR_EXECUTABLE));
        assert_eq!(to_input(&Form { args: "[1]".into(), ..form() }), Err(s::ERR_ARGS));
        assert_eq!(to_input(&Form { args: "cli.js".into(), ..form() }), Err(s::ERR_ARGS));
        assert_eq!(to_input(&Form { config_root: "relative".into(), ..form() }), Err(s::ERR_CONFIG_ROOT));
        assert_eq!(to_input(&Form { env: "NO_SEPARATOR".into(), ..form() }), Err(s::ERR_ENV));
        assert_eq!(to_input(&Form { env: "A=".into(), ..form() }), Err(s::ERR_ENV));
    }
}
