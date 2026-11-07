//! 自定义字体 and 自定义主题 (original `CustomFontsSettings.tsx`, `CustomThemesSettings.tsx`;
//! 08 A5, A13): each folder with its files and what is wrong with them.

use gpui_kit::{
    AnyElement, Context, IntoElement, ParentElement as _, PathPromptOptions, Styled as _,
    assets::IconName,
    base::{h_flex, v_flex},
    component::{Sizable as _, button::Button},
    prelude::FluentBuilder as _,
};
use thinkrail_core::FontFamily;
use thinkrail_ui::{
    Roles, ThinkrailTheme,
    components::label,
    metrics::{space, text},
    theme::UserThemeIssueKind,
};

use super::{
    super::rows::{group, notice},
    AppearancePage,
};
use crate::{
    appearance::{AppearanceState, FileProblem, FontProblem, family_name},
    strings::settings_appearance as s,
};

fn font_problem(problem: &FontProblem) -> String {
    match problem {
        FontProblem::File(FileProblem::TooLarge) => s::FONT_TOO_LARGE.to_owned(),
        FontProblem::File(FileProblem::TooMany) => s::FONT_TOO_MANY.to_owned(),
        FontProblem::File(FileProblem::Unreadable(detail)) => {
            format!("{}（{detail}）", s::FONT_UNREADABLE)
        }
        FontProblem::File(FileProblem::Collection) => s::FONT_COLLECTION.to_owned(),
        FontProblem::File(FileProblem::Unsupported) => s::FONT_UNSUPPORTED.to_owned(),
        FontProblem::Invalid(detail) => format!("{}（{detail}）", s::FONT_INVALID),
    }
}

fn theme_problem(kind: UserThemeIssueKind) -> &'static str {
    match kind {
        UserThemeIssueKind::TooLarge => s::THEME_TOO_LARGE,
        UserThemeIssueKind::TooMany => s::THEME_TOO_MANY,
        UserThemeIssueKind::Unreadable => s::THEME_UNREADABLE,
        UserThemeIssueKind::InvalidJson => s::THEME_INVALID_JSON,
        UserThemeIssueKind::InvalidTheme => s::THEME_INVALID,
        UserThemeIssueKind::IdTooLong => s::THEME_ID_TOO_LONG,
        UserThemeIssueKind::DuplicateId => s::THEME_DUPLICATE_ID,
    }
}

/// One file line: its name, then what it holds or what is wrong.
fn file_line(name: &str, detail: AnyElement, r: &Roles) -> AnyElement {
    h_flex()
        .gap(space::S12)
        .py(space::S8)
        .border_b_1()
        .border_color(r.border_muted)
        .child(label(text::CODE_SMALL, name.to_owned()).flex_none())
        .child(detail)
        .into_any_element()
}

impl AppearancePage {
    pub(super) fn custom_fonts(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let state = cx.global::<AppearanceState>();
        let prefs = &state.prefs;
        let in_use = |family: &str| {
            [&prefs.interface_font, &prefs.code_font]
                .iter()
                .any(|choice| {
                    choice
                        .family
                        .iter()
                        .chain(choice.fallbacks.iter())
                        .any(|f| {
                            matches!(f, FontFamily::Name(_))
                                && family_name(f).eq_ignore_ascii_case(family)
                        })
                })
        };
        let files: Vec<AnyElement> = state
            .fonts
            .iter()
            .map(|font| {
                let detail = match &font.face {
                    Ok(face) => h_flex()
                        .gap(space::S8)
                        .child(label(text::UI, face.family.clone()))
                        .when(face.monospace, |d| {
                            d.child(label(text::CAPTION, s::PICKER_MONO).text_color(r.text_subtle))
                        })
                        .when(in_use(&face.family), |d| {
                            d.child(
                                label(text::CAPTION, s::CUSTOM_FONTS_IN_USE).text_color(r.primary),
                            )
                        })
                        .into_any_element(),
                    Err(problem) => label(text::METADATA, font_problem(problem))
                        .text_color(r.feedback_error)
                        .into_any_element(),
                };
                file_line(&font.file_name, detail, &r)
            })
            .collect();
        let added: Vec<AnyElement> = state
            .added
            .iter()
            .map(|added| match &added.outcome {
                Ok(_) => notice(s::custom_fonts_added(&added.file_name), false, &r),
                Err(problem) => notice(
                    format!("{}：{}", added.file_name, font_problem(problem)),
                    true,
                    &r,
                ),
            })
            .collect();
        let fonts_dir = state.fonts_dir.clone();
        let choose = Button::new("custom-fonts-choose")
            .outline()
            .small()
            .icon(IconName::Plus)
            .label(s::CUSTOM_FONTS_CHOOSE)
            .on_click(|_, _, cx| {
                let picked = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: true,
                    prompt: None,
                });
                cx.spawn(async move |cx| {
                    if let Ok(Ok(Some(paths))) = picked.await {
                        cx.update(|cx| AppearanceState::add_fonts(paths, cx));
                    }
                })
                .detach();
            });
        group(
            "appearance-custom-fonts",
            s::CUSTOM_FONTS_TITLE,
            Some(s::CUSTOM_FONTS_DESCRIPTION),
            &r,
        )
        .child(
            h_flex()
                .gap(space::S8)
                .items_center()
                .py(space::S12)
                .child(choose)
                .child(label(text::METADATA, s::CUSTOM_FONTS_HINT).text_color(r.text_subtle)),
        )
        .children(added)
        .children(files)
        .child(self.folder_row("fonts", s::CUSTOM_FONTS_FOLDER, fonts_dir, &r, cx))
        .child(
            label(text::METADATA, s::CUSTOM_FONTS_REMOVE_HINT)
                .text_color(r.text_subtle)
                .pt(space::S8),
        )
        .into_any_element()
    }

    pub(super) fn custom_themes(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let r = ThinkrailTheme::global(cx).roles;
        let state = cx.global::<AppearanceState>();
        let issues: Vec<AnyElement> = state
            .catalog
            .user_issues()
            .iter()
            .map(|issue| {
                let detail = v_flex()
                    .child(
                        label(text::METADATA, theme_problem(issue.kind))
                            .text_color(r.feedback_error),
                    )
                    .children(
                        issue
                            .detail
                            .clone()
                            .map(|d| label(text::CODE_SMALL, d).text_color(r.text_subtle)),
                    )
                    .into_any_element();
                file_line(&issue.file_name, detail, &r)
            })
            .collect();
        let themes_dir = state.themes_dir.clone();
        group("appearance-custom-themes", s::CUSTOM_THEMES_TITLE, None, &r)
            .child(
                label(text::METADATA, s::CUSTOM_THEMES_STEPS)
                    .text_color(r.text_muted)
                    .py(space::S12),
            )
            .child(self.folder_row("themes", s::CUSTOM_THEMES_FOLDER, themes_dir, &r, cx))
            .children(issues)
            .into_any_element()
    }
}
