//! Appearance at runtime (native-multi-agent 08): one app global holding the stored preferences,
//! the theme catalog (bundled + `<data>/themes`), the user's font files (`<data>/fonts`) and the
//! system appearance. Every change is applied at once to every window (A7) and then stored; a
//! failed store restores the last stored value.

mod files;
mod select;
mod typeset;
mod watch;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui_kit::{App, BorrowAppContext as _, Global, WindowAppearance};
use thinkrail_core::{AppPrefService, AppearancePrefs, Error, ErrorCode as _, ThemeMode};
use thinkrail_ui::{
    Appearance, ThemeCatalog, ThinkrailTheme,
    fonts::{self, FaceInfo},
    theme::{FileIssue, SystemThemes},
};
use tracing::{debug, warn};

pub use files::FileProblem;
pub use select::{
    ModeChoice, ThemeView, select_contrast, select_fixed, select_mode, select_system,
};
pub use typeset::{Families, bundled_face, family_name, role_font};
pub use watch::start as watch;

/// One file in `<data>/fonts`.
#[derive(Debug, Clone)]
pub struct UserFont {
    pub file_name: String,
    pub face: Result<FaceInfo, FontProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontProblem {
    File(FileProblem),
    /// The parser's detail.
    Invalid(String),
}

/// The result of adding one picked file.
#[derive(Debug, Clone)]
pub struct Added {
    pub file_name: String,
    pub outcome: Result<String, FontProblem>,
}

pub struct AppearanceState {
    pub prefs: AppearancePrefs,
    /// What the store last confirmed; restored when a write fails.
    stored: AppearancePrefs,
    pub catalog: ThemeCatalog,
    pub fonts: Vec<UserFont>,
    pub families: Families,
    /// Installed system families, sorted case-insensitively and deduplicated.
    pub installed: Arc<Vec<String>>,
    pub system: Appearance,
    pub themes_dir: PathBuf,
    pub fonts_dir: PathBuf,
    /// Font file bytes already handed to the text system (it cannot unregister them).
    registered: HashSet<(String, u64)>,
    pub(crate) service: Arc<dyn AppPrefService>,
    /// The outcome of the last "选择文件…", shown under the button.
    pub added: Vec<Added>,
    /// A failed write's message, shown on the page until the next change.
    pub error: Option<&'static str>,
}

impl Global for AppearanceState {}

pub fn system_appearance(appearance: WindowAppearance) -> Appearance {
    match appearance {
        WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
    }
}

fn installed_families(cx: &App) -> Vec<String> {
    let mut names = cx.text_system().all_font_names();
    names.retain(|name| !name.starts_with('.') && thinkrail_core::is_font_family_name(name));
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

fn theme_files(dir: &Path) -> Vec<(String, Result<String, FileIssue>)> {
    files::read_themes(dir)
        .into_iter()
        .map(|file| {
            let contents = file.contents.map_err(|problem| match problem {
                FileProblem::TooLarge => FileIssue::TooLarge,
                FileProblem::TooMany => FileIssue::TooMany,
                FileProblem::Unreadable(detail) => FileIssue::Unreadable(detail),
                FileProblem::Collection | FileProblem::Unsupported => {
                    FileIssue::Unreadable(String::new())
                }
            });
            (file.file_name, contents)
        })
        .collect()
}

/// What a folder scan found, read off the UI thread.
pub(crate) struct Scan {
    themes: Vec<(String, Result<String, FileIssue>)>,
    fonts: Vec<(String, Result<Vec<u8>, FileProblem>)>,
}

pub(crate) fn scan(themes_dir: &Path, fonts_dir: &Path) -> Scan {
    Scan {
        themes: theme_files(themes_dir),
        fonts: files::read_fonts(fonts_dir)
            .into_iter()
            .map(|f| (f.file_name, f.bytes))
            .collect(),
    }
}

impl AppearanceState {
    /// Startup (A7): read the stored value and both folders, before the window opens.
    pub fn init(service: Arc<dyn AppPrefService>, data_dir: &Path, cx: &mut App) {
        let prefs = match service.app_prefs() {
            Ok(stored) => stored.appearance,
            Err(error) => {
                warn!(
                    code = error.code(),
                    "appearance not read; defaults are used"
                );
                AppearancePrefs::default()
            }
        };
        let themes_dir = data_dir.join("themes");
        let fonts_dir = data_dir.join("fonts");
        for dir in [&themes_dir, &fonts_dir] {
            if let Err(error) = std::fs::create_dir_all(dir) {
                warn!(%error, "appearance folder not created");
            }
        }
        let catalog = match ThemeCatalog::bundled() {
            Ok(catalog) => catalog.clone(),
            Err(error) => {
                // The bundled manifests are compiled in and pinned by tests.
                warn!(error, "bundled themes unavailable");
                return;
            }
        };
        let installed = Arc::new(installed_families(cx));
        let mut state = Self {
            stored: prefs.clone(),
            prefs,
            catalog,
            fonts: Vec::new(),
            families: Families::default(),
            installed,
            system: system_appearance(cx.window_appearance()),
            themes_dir: themes_dir.clone(),
            fonts_dir: fonts_dir.clone(),
            registered: HashSet::new(),
            service,
            added: Vec::new(),
            error: None,
        };
        state.absorb(scan(&themes_dir, &fonts_dir), cx);
        cx.set_global(state);
        Self::apply(cx);
    }

    /// Takes a folder scan: rebuilds the catalog, registers new font files, rebuilds families.
    pub(crate) fn absorb(&mut self, scan: Scan, cx: &mut App) {
        match ThemeCatalog::bundled() {
            Ok(bundled) => self.catalog = bundled.with_user_themes(scan.themes),
            Err(error) => warn!(error, "bundled themes unavailable"),
        }
        self.fonts = scan
            .fonts
            .into_iter()
            .map(|(file_name, bytes)| {
                let face = bytes.map_err(FontProblem::File).and_then(|bytes| {
                    let face = fonts::inspect(&bytes).map_err(|error| match error {
                        fonts::InspectError::Collection => {
                            FontProblem::File(FileProblem::Collection)
                        }
                        fonts::InspectError::Invalid(detail) => FontProblem::Invalid(detail),
                    })?;
                    let key = (file_name.clone(), content_key(&bytes));
                    if !self.registered.contains(&key) {
                        match fonts::register(cx, bytes) {
                            Ok(()) => {
                                self.registered.insert(key);
                            }
                            Err(error) => {
                                return Err(FontProblem::Invalid(format!("{error:#}")));
                            }
                        }
                    }
                    Ok(face)
                });
                UserFont { file_name, face }
            })
            .collect();
        let custom: Vec<&str> = self
            .fonts
            .iter()
            .filter_map(|f| f.face.as_ref().ok().map(|face| face.family.as_str()))
            .collect();
        self.families = Families::new(self.installed.iter().map(String::as_str), custom);
    }

    pub fn view(&self) -> ThemeView {
        ThemeView::new(&self.catalog, &self.prefs, self.system)
    }

    /// The theme on screen for these preferences.
    fn current_theme(&self) -> Arc<ThinkrailTheme> {
        let pair = self.prefs.system_theme_pair.as_ref();
        let system = match (self.prefs.theme_mode, pair) {
            (ThemeMode::System, Some(pair)) => Some(SystemThemes {
                light: &pair.light,
                dark: &pair.dark,
                appearance: self.system,
            }),
            (ThemeMode::System | ThemeMode::Fixed, _) => None,
        };
        self.catalog.resolve(&self.prefs.theme, system).theme
    }

    /// Installs the resolved theme and fonts and redraws every window.
    pub fn apply(cx: &mut App) {
        let Some(state) = cx.try_global::<Self>() else {
            return;
        };
        let theme = state.current_theme();
        let typography = typeset::typography(&state.prefs, &state.families);
        thinkrail_ui::Typography::set(typography, cx);
        theme.install(None, cx);
        cx.refresh_windows();
    }

    /// Edits the current preferences and saves the result (see [`Self::save`]).
    pub fn edit(message: &'static str, cx: &mut App, change: impl FnOnce(&mut AppearancePrefs)) {
        let Some(state) = cx.try_global::<Self>() else {
            return;
        };
        let mut next = state.prefs.clone();
        change(&mut next);
        Self::save(next, message, cx);
    }

    /// Applies `next` at once, then stores it in the background; a failed store restores the
    /// last stored value and keeps `message` on the page.
    pub fn save(next: AppearancePrefs, message: &'static str, cx: &mut App) {
        let service = {
            let state = cx.global_mut::<Self>();
            if state.prefs == next {
                return;
            }
            state.prefs = next.clone();
            state.error = None;
            Arc::clone(&state.service)
        };
        Self::apply(cx);
        let job = cx
            .background_executor()
            .spawn(async move { service.set_appearance(&next) });
        cx.spawn(async move |cx| {
            let result: Result<_, Error> = job.await;
            cx.update(|cx| {
                let state = cx.global_mut::<Self>();
                match result {
                    Ok(stored) => state.stored = stored.appearance,
                    Err(error) => {
                        warn!(code = error.code(), "appearance not stored");
                        state.prefs = state.stored.clone();
                        state.error = Some(message);
                    }
                }
                Self::apply(cx);
            });
        })
        .detach();
    }

    /// Copies picked font files into `<data>/fonts` off the UI thread, then rescans.
    pub fn add_fonts(paths: Vec<PathBuf>, cx: &mut App) {
        let state = cx.global::<Self>();
        let (themes_dir, fonts_dir) = (state.themes_dir.clone(), state.fonts_dir.clone());
        let job = cx.background_executor().spawn(async move {
            let added: Vec<Added> = paths
                .iter()
                .map(|path| {
                    let outcome = files::copy_font(path, &fonts_dir, |bytes| {
                        fonts::inspect(bytes)
                            .map(drop)
                            .map_err(|e| format!("{e:?}"))
                    })
                    .map_err(|problem| match problem {
                        files::CopyProblem::File(problem) => FontProblem::File(problem),
                        files::CopyProblem::Invalid(detail) => FontProblem::Invalid(detail),
                    });
                    Added {
                        file_name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        outcome,
                    }
                })
                .collect();
            (added, scan(&themes_dir, &fonts_dir))
        });
        cx.spawn(async move |cx| {
            let (added, scan) = job.await;
            cx.update(|cx| {
                cx.update_global::<Self, _>(|state, cx| {
                    state.absorb(scan, cx);
                    state.added = added;
                });
                Self::apply(cx);
            });
        })
        .detach();
    }

    /// The system's appearance changed (A6): only system mode redraws.
    pub fn set_system(appearance: Appearance, cx: &mut App) {
        let state = cx.global_mut::<Self>();
        if state.system == appearance {
            return;
        }
        state.system = appearance;
        debug!(?appearance, "system appearance changed");
        Self::apply(cx);
    }
}

/// Identifies a font file's contents without keeping them.
fn content_key(bytes: &[u8]) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
