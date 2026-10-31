//! MCP server settings: global entries and project-effective overrides.

use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, InteractiveElement as _, ParentElement as _, Render,
    Styled as _, Subscription, Window,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{Disableable as _, button::{Button, ButtonVariants as _}, input::{Input, InputEvent, InputState, Textarea, TextareaState}, switch::Switch},
    prelude::FluentBuilder as _, px,
};
use serde_json::Value;
use thinkrail_core::{EffectiveMcpServer, EnvValue, McpOrigin, McpScope, McpServer, McpServerId, McpServerInput, McpTransport, ProjectId};
use thinkrail_ui::{ThinkrailTheme, components::{IconButton, Segment, label}, metrics::{radius, space, text}};

use crate::{state::{AppEvent, AppState, run}, strings::settings_mcp as s};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewScope { Global, Project }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportKind { Stdio, Http }

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportPreview {
    valid: Vec<McpServerInput>,
    invalid: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Form {
    pub name: String,
    pub transport: TransportKind,
    pub command: String,
    pub args: String,
    pub url: String,
    pub env: Vec<EnvValue>,
    pub headers: Vec<EnvValue>,
    pub description: String,
    pub enabled: bool,
    pub scope: McpScope,
}

fn validate_pairs(values: &[EnvValue], error: &'static str) -> Result<Vec<EnvValue>, &'static str> {
    if values.iter().any(|pair| pair.name.trim().is_empty()) { return Err(error); }
    Ok(values.iter().map(|pair| EnvValue { name: pair.name.trim().to_owned(), value: pair.value.clone() }).collect())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 32 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}


fn to_input(form: &Form) -> Result<McpServerInput, &'static str> {
    let name = form.name.trim();
    if name.is_empty() { return Err(s::NAME_REQUIRED) }
    if !valid_name(name) { return Err(s::NAME_INVALID) }
    let transport = match form.transport {
        TransportKind::Stdio => {
            let command = form.command.trim();
            if command.is_empty() { return Err(s::COMMAND_REQUIRED) }
            McpTransport::Stdio {
                command: command.to_owned(),
                args: form.args.lines().map(str::trim).filter(|line| !line.is_empty()).map(ToOwned::to_owned).collect(),
                env: validate_pairs(&form.env, s::ENV_INVALID)?,
            }
        }
        TransportKind::Http => {
            let url = form.url.trim();
            validate_url(url)?;
            McpTransport::Http { url: url.to_owned(), headers: validate_pairs(&form.headers, s::HEADERS_INVALID)? }
        }
    };
    Ok(McpServerInput {
        name: name.to_owned(),
        scope: form.scope.clone(),
        transport,
        enabled: form.enabled,
        description: (!form.description.trim().is_empty()).then(|| form.description.trim().to_owned()),
    })
}

fn import_items(value: Value) -> Result<Vec<(String, Value)>, &'static str> {
    let object = value.as_object().ok_or(s::IMPORT_INVALID)?;
    let source = match object.get("mcpServers") {
        Some(servers) => servers.as_object().ok_or(s::IMPORT_INVALID)?,
        None => object,
    };
    if source.is_empty() { return Err(s::IMPORT_EMPTY) }
    Ok(source.iter().map(|(name, value)| (name.clone(), value.clone())).collect())
}

fn import_transport(value: &Value) -> Result<McpTransport, &'static str> {
    let object = value.as_object().ok_or(s::IMPORT_ITEM_INVALID)?;
    if let Some(command) = object.get("command").and_then(Value::as_str) {
        let command = command.trim();
        if command.is_empty() { return Err(s::COMMAND_REQUIRED) }
        let args = object.get("args").map_or_else(|| Ok(Vec::new()), |value| value.as_array().ok_or(s::IMPORT_ITEM_INVALID).and_then(|args| args.iter().map(|arg| arg.as_str().map(ToOwned::to_owned)).collect::<Option<Vec<_>>>().ok_or(s::IMPORT_ITEM_INVALID)))?;
        let env = object.get("env").map(|value| parse_import_pairs(value, s::ENV_INVALID)).transpose()?.unwrap_or_default();
        return Ok(McpTransport::Stdio { command: command.to_owned(), args, env });
    }
    if let Some(url) = object.get("url").and_then(Value::as_str) {
        validate_url(url)?;
        let headers = object.get("headers").map(|value| parse_import_pairs(value, s::HEADERS_INVALID)).transpose()?.unwrap_or_default();
        return Ok(McpTransport::Http { url: url.to_owned(), headers });
    }
    Err(s::IMPORT_ITEM_INVALID)
}

fn parse_import_pairs(value: &Value, error: &'static str) -> Result<Vec<EnvValue>, &'static str> {
    let object = value.as_object().ok_or(error)?;
    object.iter().map(|(name, value)| { let name = name.trim(); if name.is_empty() { return Err(error) }; value.as_str().filter(|value| !value.is_empty()).map(|value| EnvValue { name: name.to_owned(), value: value.to_owned() }).ok_or(error) }).collect()
}


fn validate_url(url: &str) -> Result<(), &'static str> {
    let parsed = url::Url::parse(url).map_err(|_| s::URL_REQUIRED)?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none()
        || !parsed.username().is_empty() || parsed.password().is_some()
    {
        return Err(s::URL_REQUIRED);
    }
    Ok(())
}

fn parse_import(raw: &str, scope: McpScope) -> Result<ImportPreview, &'static str> {
    let value: Value = serde_json::from_str(raw).map_err(|_| s::IMPORT_INVALID)?;
    let items = import_items(value)?;
    let mut valid = Vec::new();
    let mut invalid = 0;
    for (name, value) in items {
        if !valid_name(&name) || name.len() > 32 { invalid += 1; continue }
        let transport = match import_transport(&value) { Ok(transport) => transport, Err(_) => { invalid += 1; continue } };
        let object = value.as_object().ok_or(s::IMPORT_ITEM_INVALID)?;
        let description = object.get("description").and_then(Value::as_str).map(ToOwned::to_owned);
        let enabled = object.get("enabled").and_then(Value::as_bool).unwrap_or(true)
            && !object.get("disabled").and_then(Value::as_bool).unwrap_or(false);
        valid.push(McpServerInput { name, scope: scope.clone(), transport, enabled, description });
    }
    if valid.is_empty() && invalid == 0 { return Err(s::IMPORT_EMPTY) }
    Ok(ImportPreview { valid, invalid })
}

struct PairFields {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

struct Editor {
    id: Option<McpServerId>,
    revision: u64,
    name: Entity<InputState>,
    command: Entity<InputState>,
    url: Entity<InputState>,
    args: Entity<TextareaState>,
    env: Vec<PairFields>,
    headers: Vec<PairFields>,
    description: Entity<InputState>,
    transport: TransportKind,
    enabled: bool,
    scope: McpScope,
    saving: bool,
}

pub struct McpPage {
    state: Entity<AppState>,
    scope: ViewScope,
    global: Vec<McpServer>,
    effective: Vec<EffectiveMcpServer>,
    editor: Option<Editor>,
    import: Entity<TextareaState>,
    import_preview: Option<ImportPreview>,
    loading: bool,
    load_generation: u64,
    importing: bool,
    confirm_delete: Option<(McpServerId, u64)>,
    error: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl McpPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let import = cx.new(|cx| TextareaState::new(window, cx).rows(5).placeholder(s::IMPORT_JSON));
        let subscriptions = vec![cx.subscribe(&state, |this, _, event: &AppEvent, cx| {
            if matches!(event, AppEvent::Projects | AppEvent::Workspace) {
                this.editor = None;
                this.import_preview = None;
                if this.project(cx).is_none() { this.scope = ViewScope::Global; }
                this.reload(cx);
            }
        }), cx.subscribe(&import, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.import_preview = None;
                cx.notify();
            }
        })];
        let mut page = Self { state, scope: ViewScope::Global, global: Vec::new(), effective: Vec::new(), editor: None, import, import_preview: None, loading: false, load_generation: 0, importing: false, confirm_delete: None, error: None, _subscriptions: subscriptions };
        page.reload(cx);
        page
    }

    fn project(&self, cx: &Context<Self>) -> Option<ProjectId> { self.state.read(cx).project.clone() }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_generation += 1;
        let generation = self.load_generation;
        let service = self.state.read(cx).services.mcp.clone();
        let project = self.project(cx);
        let requested_scope = self.scope;
        self.loading = true;
        match requested_scope {
            ViewScope::Global => run(cx, move || service.global(), move |this, result, cx| {
                if this.load_generation != generation { return; }
                this.loading = false;
                match result { Ok(value) => { this.global = value; this.error = None; cx.notify(); }, Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)) }
            }),
            ViewScope::Project => {
                let Some(project) = project else { self.loading = false; self.effective.clear(); return };
                let expected = project.clone();
                run(cx, move || service.effective(&project), move |this, result, cx| {
                    if this.load_generation != generation || this.scope != requested_scope || this.project(cx).as_ref() != Some(&expected) { return; }
                    this.loading = false;
                    match result { Ok(value) => { this.effective = value; this.error = None; cx.notify(); }, Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)) }
                });
            }
        }
    }
    fn begin_edit(&mut self, server: Option<McpServer>, window: &mut Window, cx: &mut Context<Self>) {
        let input = |cx: &mut Context<Self>, window: &mut Window| cx.new(|cx| InputState::new(window, cx));
        let area = |cx: &mut Context<Self>, window: &mut Window| cx.new(|cx| TextareaState::new(window, cx).rows(3));
        let scope = server.as_ref().map_or_else(|| match self.scope { ViewScope::Global => McpScope::Global, ViewScope::Project => self.project(cx).map_or(McpScope::Global, |project| McpScope::Project { project }) }, |server| server.input.scope.clone());
        let (name, transport, command, args, url, pairs, description, enabled, id, revision) = server.as_ref().map_or_else(
            || (String::new(), TransportKind::Stdio, String::new(), String::new(), String::new(), Vec::<(String, String)>::new(), String::new(), true, None, 0),
            |server| match &server.input.transport {
                McpTransport::Stdio { command, args, env } => (server.input.name.clone(), TransportKind::Stdio, command.clone(), args.join("\n"), String::new(), env.iter().map(|pair| (pair.name.clone(), pair.value.clone())).collect(), server.input.description.clone().unwrap_or_default(), server.input.enabled, Some(server.id.clone()), server.revision),
                McpTransport::Http { url, headers } => (server.input.name.clone(), TransportKind::Http, String::new(), String::new(), url.clone(), headers.iter().map(|pair| (pair.name.clone(), pair.value.clone())).collect(), server.input.description.clone().unwrap_or_default(), server.input.enabled, Some(server.id.clone()), server.revision),
            },
        );
        let mut env_pairs = Vec::new();
        let mut header_pairs = Vec::new();
        for (key, value) in pairs {
            let key_field = input(cx, window);
            let value_field = input(cx, window);
            set_input(&key_field, key, window, cx);
            set_input(&value_field, value, window, cx);
            let pair = PairFields { key: key_field, value: value_field };
            if transport == TransportKind::Stdio { env_pairs.push(pair); } else { header_pairs.push(pair); }
        }
        let editor = Editor { id, revision, name: input(cx, window), command: input(cx, window), url: input(cx, window), args: area(cx, window), env: env_pairs, headers: header_pairs, description: input(cx, window), transport, enabled, scope, saving: false };
        set_input(&editor.name, name, window, cx); set_input(&editor.command, command, window, cx); set_input(&editor.url, url, window, cx); set_area(&editor.args, args, window, cx); set_input(&editor.description, description, window, cx);
        self.editor = Some(editor); self.error = None; cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else { return };
        if editor.saving { return }
        let value = |field: &Entity<InputState>| field.read(cx).value().to_string();
        let area = |field: &Entity<TextareaState>| field.read(cx).value().to_string();
        let pairs = |fields: &[PairFields]| fields.iter().map(|pair| EnvValue { name: value(&pair.key), value: value(&pair.value) }).collect();
        let form = Form { name: value(&editor.name), transport: editor.transport, command: value(&editor.command), args: area(&editor.args), url: value(&editor.url), env: pairs(&editor.env), headers: pairs(&editor.headers), description: value(&editor.description), enabled: editor.enabled, scope: editor.scope.clone() };
        let input = match to_input(&form) { Ok(input) => input, Err(error) => { self.error = Some(error); cx.notify(); return } };
        editor.saving = true;
        let id = editor.id.clone();
        let revision = editor.revision;
        let pending_name = editor.name.clone();
        let service = self.state.read(cx).services.mcp.clone();
        run(cx, move || match id {
            Some(id) => service.update(&id, revision, input).map(drop),
            None => service.create(input).map(drop),
        }, move |this, result, cx| {
            let same_editor = this.editor.as_ref().is_some_and(|editor| editor.name == pending_name);
            match result {
                Ok(()) => {
                    if same_editor { this.editor = None; }
                    this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    this.reload(cx);
                }
                Err(error) => {
                    if same_editor && let Some(editor) = &mut this.editor { editor.saving = false; }
                    this.state.update(cx, |state, cx| state.report(&error, cx));
                }
            }
            cx.notify();
        });
    }

    fn toggle(&mut self, server: McpServer, enabled: bool, cx: &mut Context<Self>) {
        let service = self.state.read(cx).services.mcp.clone();
        let project = self.project(cx);
        let origin_is_global = self.scope == ViewScope::Project && self.effective.iter().any(|row| row.server.id == server.id && row.origin == McpOrigin::Global);
        run(cx, move || {
            if origin_is_global {
                let project = project.ok_or_else(|| thinkrail_core::Error::not_found(s::PROJECT_MISSING))?;
                service.set_override(&project, &server.id, Some(enabled))
            } else {
                let mut input = server.input.clone();
                input.enabled = enabled;
                service.update(&server.id, server.revision, input).map(drop)
            }
        }, |this, result, cx| match result {
            Ok(()) => {
                this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                this.reload(cx);
            }
            Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)),
        });
    }

    fn delete(&mut self, server: McpServer, cx: &mut Context<Self>) {
        if self.confirm_delete.as_ref() != Some(&(server.id.clone(), server.revision)) { self.confirm_delete = Some((server.id.clone(), server.revision)); cx.notify(); return }
        self.confirm_delete = None;
        let service = self.state.read(cx).services.mcp.clone();
        run(cx, move || service.delete(&server.id, server.revision), |this, result, cx| { match result { Ok(()) => { this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing)); this.reload(cx); }, Err(error) => this.state.update(cx, |state, cx| state.report(&error, cx)) } });
    }

    fn preview_import(&mut self, cx: &mut Context<Self>) {
        let raw = self.import.read(cx).value().to_string();
        let scope = match self.scope { ViewScope::Global => McpScope::Global, ViewScope::Project => self.project(cx).map_or(McpScope::Global, |project| McpScope::Project { project }) };
        match parse_import(&raw, scope) { Ok(preview) => { self.error = None; self.import_preview = Some(preview); }, Err(error) => { self.error = Some(error); self.import_preview = None; } }
        cx.notify();
    }

    fn import_valid(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.import_preview.clone() else { return };
        if self.importing || preview.valid.is_empty() { return }
        self.importing = true;
        let pending = self.import.clone();
        let service = self.state.read(cx).services.mcp.clone();
        run(cx, move || {
            let mut succeeded = 0usize;
            let mut failure = None;
            for input in preview.valid {
                match service.create(input) {
                    Ok(_) => succeeded += 1,
                    Err(error) => failure = Some(error),
                }
            }
            Ok((succeeded, failure))
        }, move |this, result, cx| {
            this.importing = false;
            if let Ok((succeeded, failure)) = result {
                if this.import == pending { this.import_preview = None; }
                if succeeded > 0 {
                    this.state.update(cx, |_, cx| cx.emit(AppEvent::Routing));
                    this.reload(cx);
                }
                if let Some(error) = failure {
                    this.state.update(cx, |state, cx| state.report(&error, cx));
                }
            }
            cx.notify();
        });
    }

    fn row(&self, server: McpServer, origin: McpOrigin, enabled: bool, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles;
        let id = server.id.clone();
        let edit = server.clone();
        let toggle = server.clone();
        let delete = server.clone();
        let project_global = self.scope == ViewScope::Project && origin != McpOrigin::Project;
        let transport = match &server.input.transport {
            McpTransport::Stdio { command, .. } => format!("{} · {}", s::STDIO, command),
            McpTransport::Http { url, .. } => format!("{} · {}", s::HTTP, url),
        };
        let origin_label = match origin {
            McpOrigin::Global if self.scope == ViewScope::Global => s::GLOBAL,
            McpOrigin::Global => s::FROM_GLOBAL,
            McpOrigin::Project => s::PROJECT_SCOPE,
            McpOrigin::GlobalOverridden => s::OVERRIDDEN,
        };
        let confirming = self.confirm_delete.as_ref() == Some(&(server.id.clone(), server.revision));
        v_flex().gap(space::S4).py(space::S12).border_b_1().border_color(r.border_muted)
            .child(h_flex().id(format!("mcp-{id}")).gap(space::S8)
                .child(v_flex().flex_1().min_w(px(0.))
                    .child(h_flex().gap(space::S6)
                        .child(label(text::UI, server.input.name.clone()))
                        .child(label(text::METADATA, origin_label).text_color(r.text_subtle)))
                    .child(label(text::METADATA, transport).text_color(r.text_subtle))
                    .when_some(server.input.description.clone(), |d, description| d
                        .child(label(text::METADATA, description).text_color(r.text_muted))))
                .child(Switch::new(format!("mcp-enabled-{id}")).checked(enabled).color(r.feedback_success)
                    .disabled(origin == McpOrigin::GlobalOverridden)
                    .accessibility_label(server.input.name.clone())
                    .on_click(cx.listener(move |this, enabled: &bool, _, cx| this.toggle(toggle.clone(), *enabled, cx))))
                .when(!project_global, |d| d
                    .child(IconButton::new(format!("mcp-edit-{id}"), IconName::Pencil).tooltip(s::EDIT)
                        .on_click(cx.listener(move |this, _, window, cx| this.begin_edit(Some(edit.clone()), window, cx))))
                    .child(if confirming {
                        Button::new(format!("mcp-confirm-delete-{id}")).danger().label(s::CONFIRM_DELETE)
                            .on_click(cx.listener(move |this, _, _, cx| this.delete(delete.clone(), cx))).into_any_element()
                    } else {
                        IconButton::new(format!("mcp-delete-{id}"), IconName::Trash).tooltip(s::DELETE)
                            .on_click(cx.listener(move |this, _, _, cx| this.delete(delete.clone(), cx))).into_any_element()
                    })))
            .when(confirming, |d| d.child(label(text::METADATA, format!("{} {}", server.input.name, s::REMOVE_WARNING)).text_color(r.feedback_warning)))
    }

    fn editor(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let r = ThinkrailTheme::global(cx).roles; let Some(editor) = &self.editor else { return v_flex().into_any_element() };
        let field = |title: &'static str, input: &Entity<InputState>| v_flex().gap(space::S2).child(label(text::TITLE_COMPACT, title).text_color(r.text_muted)).child(Input::new(input));
        let area = |title: &'static str, input: &Entity<TextareaState>| v_flex().gap(space::S2).child(label(text::TITLE_COMPACT, title).text_color(r.text_muted)).child(Textarea::new(input));
        let pairs = |title: &'static str, fields: &Vec<PairFields>| {
            v_flex().gap(space::S4).child(label(text::TITLE_COMPACT, title).text_color(r.text_muted))
                .children(fields.iter().enumerate().map(|(index, pair)| {
                    h_flex().gap(space::S4).child(Input::new(&pair.key)).child(Input::new(&pair.value)).child(
                        IconButton::new(format!("mcp-remove-{title}-{index}"), IconName::Trash).on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = &mut this.editor {
                                let target = if title == s::ENV { &mut editor.env } else { &mut editor.headers };
                                if index < target.len() { target.remove(index); cx.notify(); }
                            }
                        })),
                    )
                }))
                .child(Button::new(format!("mcp-add-{title}")).ghost().label(s::ADD).on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(editor) = &mut this.editor {
                        let pair = PairFields { key: cx.new(|cx| InputState::new(window, cx)), value: cx.new(|cx| InputState::new(window, cx)) };
                        if title == s::ENV { editor.env.push(pair); } else { editor.headers.push(pair); }
                        cx.notify();
                    }
                })))
        };
        v_flex().gap(space::S8).p(space::S12).rounded(radius::MD).bg(r.container_content_bg)
            .child(label(text::TITLE_CARD, if editor.id.is_some() { s::EDIT } else { s::ADD }))
            .child(field(s::NAME, &editor.name))
            .child(label(text::METADATA, s::NAME_HINT).text_color(r.text_subtle))
            .child(Segment::new("mcp-transport", [s::STDIO, s::HTTP], usize::from(editor.transport == TransportKind::Http))
                .on_select(cx.listener(|this, index: &usize, _, cx| {
                    if let Some(editor) = &mut this.editor {
                        editor.transport = if *index == 0 { TransportKind::Stdio } else { TransportKind::Http };
                        cx.notify();
                    }
                })))
            .when(editor.transport == TransportKind::Stdio, |d| d
                .child(field(s::COMMAND, &editor.command))
                .child(area(s::ARGS, &editor.args))
                .child(pairs(s::ENV, &editor.env)))
            .when(editor.transport == TransportKind::Http, |d| d
                .child(field(s::URL, &editor.url))
                .child(label(text::METADATA, s::URL_HINT).text_color(r.text_subtle))
                .child(pairs(s::HEADERS, &editor.headers)))
            .child(label(text::METADATA, s::VALUE_HINT).text_color(r.text_subtle))
            .child(field(s::DETAILS, &editor.description))
            .child(Segment::new("mcp-enabled", [s::ENABLED, s::DISABLED], usize::from(!editor.enabled))
                .on_select(cx.listener(|this, index: &usize, _, cx| {
                    if let Some(editor) = &mut this.editor { editor.enabled = *index == 0; cx.notify(); }
                })))
            .child(h_flex().gap(space::S8).justify_end()
                .child(Button::new("mcp-cancel").ghost().label(s::CANCEL).disabled(editor.saving)
                    .on_click(cx.listener(|this, _, _, cx| { this.editor = None; cx.notify(); })))
                .child(Button::new("mcp-save").primary().label(s::SAVE).disabled(editor.saving)
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx)))))
            .into_any_element()
    }
}

fn set_input(field: &Entity<InputState>, value: String, window: &mut Window, cx: &mut Context<impl Sized>) { field.update(cx, |input, cx| input.set_value(value, window, cx)); }
fn set_area(field: &Entity<TextareaState>, value: String, window: &mut Window, cx: &mut Context<impl Sized>) { field.update(cx, |input, cx| input.set_value(value, window, cx)); }

impl Render for McpPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let r = ThinkrailTheme::global(cx).roles;
        let project_label = self.state.read(cx).current_project().map(|project| format!("{}{}", s::PROJECT, project.name));
        let rows: Vec<_> = match self.scope {
            ViewScope::Global => self.global.clone().into_iter().map(|server| {
                let enabled = server.input.enabled;
                self.row(server, McpOrigin::Global, enabled, cx).into_any_element()
            }).collect(),
            ViewScope::Project => self.effective.clone().into_iter().map(|entry| {
                self.row(entry.server, entry.origin, entry.enabled, cx).into_any_element()
            }).collect(),
        };
        let empty = rows.is_empty();
        let mut scopes = vec![s::GLOBAL.to_owned()];
        if let Some(project) = project_label { scopes.push(project); }
        v_flex().id("settings-mcp").gap(space::S12)
            .child(h_flex().child(label(text::HEADING_MD, s::TITLE).flex_1())
                .child(Button::new("mcp-add").primary().icon(IconName::Plus).label(s::ADD)
                    .on_click(cx.listener(|this, _, window, cx| this.begin_edit(None, window, cx)))))
            .child(label(text::METADATA, s::DESCRIPTION).text_color(r.text_subtle))
            .child(Segment::new("mcp-scope", scopes, usize::from(self.scope == ViewScope::Project))
                .on_select(cx.listener(|this, index: &usize, _, cx| {
                    if *index == 1 && this.project(cx).is_none() { return; }
                    this.scope = if *index == 0 { ViewScope::Global } else { ViewScope::Project };
                    this.import_preview = None;
                    this.editor = None;
                    this.confirm_delete = None;
                    this.reload(cx);
                    cx.notify();
                })))
            .when(self.editor.is_some(), |d| d.child(self.editor(cx)))
            .children(rows)
            .when(empty && !self.loading, |d| d.child(label(text::METADATA, s::EMPTY).text_color(r.text_subtle)))
            .child(h_flex().gap(space::S8).children([
                (s::CONTEXT7, "https://mcp.context7.com/mcp", "context7"),
                (s::DEEPWIKI, "https://mcp.deepwiki.com/mcp", "deepwiki"),
            ].into_iter().map(|(title, url, name)| {
                Button::new(format!("mcp-preset-{name}")).ghost().label(title)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.begin_edit(None, window, cx);
                        if let Some(editor) = &mut this.editor {
                            editor.transport = TransportKind::Http;
                            set_input(&editor.name, name.to_owned(), window, cx);
                            set_input(&editor.url, url.to_owned(), window, cx);
                        }
                        cx.notify();
                    }))
            })))
            .child(v_flex().gap(space::S8).border_t_1().border_color(r.border_muted).pt(space::S12)
                .child(label(text::TITLE_SECTION, s::IMPORT_JSON))
                .child(Textarea::new(&self.import))
                .child(h_flex().gap(space::S8)
                    .child(Button::new("mcp-preview").ghost().label(s::PREVIEW)
                        .on_click(cx.listener(|this, _, _, cx| this.preview_import(cx))))
                    .child(Button::new("mcp-import").primary().label(s::IMPORT).disabled(self.importing)
                        .on_click(cx.listener(|this, _, _, cx| this.import_valid(cx)))))
                .when_some(self.import_preview.as_ref(), |d, preview| d
                    .child(label(text::METADATA, format!("{}：{} / {}", s::PREVIEW, preview.valid.len(), preview.invalid)))
                    .children(preview.valid.iter().map(|input| label(text::METADATA, input.name.clone())))))
            .when_some(self.error, |d, error| d.child(label(text::METADATA, error).text_color(r.feedback_error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> McpScope { McpScope::Global }
    fn form() -> Form { Form { name: "demo_server".into(), transport: TransportKind::Stdio, command: "node".into(), args: "server.js\n--stdio".into(), url: String::new(), env: vec![EnvValue { name: "TOKEN".into(), value: "$TOKEN".into() }], headers: Vec::new(), description: String::new(), enabled: true, scope: scope() } }

    #[test]
    fn structured_form_preserves_argument_order_and_environment_references() {
        let input = to_input(&form()).unwrap();
        let McpTransport::Stdio { command, args, env } = input.transport else { panic!() };
        assert_eq!(command, "node");
        assert_eq!(args, ["server.js", "--stdio"]);
        assert_eq!(env, [EnvValue { name: "TOKEN".into(), value: "$TOKEN".into() }]);
    }

    #[test]
    fn rejects_invalid_name_and_pairs() {
        assert_eq!(to_input(&Form { name: "bad name".into(), ..form() }), Err(s::NAME_INVALID));
        assert_eq!(to_input(&Form { env: vec![EnvValue { name: " ".into(), value: "secret".into() }], ..form() }), Err(s::ENV_INVALID));
    }

    #[test]
    fn import_keeps_only_valid_items() {
        let raw = r#"{"mcpServers":{"ok":{"command":"node","args":[],"env":{}},"bad name":{"command":"node","args":[],"env":{}}}}"#;
        let preview = parse_import(raw, scope());
        assert_eq!(preview.as_ref().map(|p| p.valid.len()), Ok(1));
        assert_eq!(preview.as_ref().map(|p| p.invalid), Ok(1));
    }

    #[test]
    fn import_forces_current_scope_and_keeps_disabled_servers() {
        let scope = McpScope::Project { project: "current".into() };
        let preview = parse_import(r#"{"local":{"command":"node","disabled":true,"scope":"global"},"bad-url":{"url":"https://user:password@example.com/mcp"},"good-url":{"url":"https://example.com/mcp"}}"#, scope.clone()).unwrap();
        assert_eq!(preview.valid.len(), 2);
        assert_eq!(preview.invalid, 1);
        assert!(preview.valid.iter().all(|input| input.scope == scope));
        let local = preview.valid.iter().find(|input| input.name == "local").unwrap();
        assert!(!local.enabled);
        assert!(matches!(&local.transport, McpTransport::Stdio { args, .. } if args.is_empty()));
        assert_eq!(parse_import(r#"{"mcpServers":[]}"#, scope), Err(s::IMPORT_INVALID));
    }
}
