//! Stateless, props-driven building blocks of the ThinkRail shell. They read only the installed
//! [`ThinkrailTheme`]; clicks are reported through the handlers they are given.

mod buttons;
mod panel;
mod tabs;
mod top_bar;

use gpui_kit::{App, ClickEvent, ParentElement, SharedString, Styled, Window, div};

use crate::{fonts, metrics::TextStyle, theme::ThinkrailTheme};

pub use buttons::{ActivityItem, IconButton};
pub use panel::{PanelHeader, TreeRow};
pub use tabs::{Segment, Tab, TabStrip};
pub use top_bar::{Breadcrumb, Status, StatusDot};

type Handler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type SelectHandler = Box<dyn Fn(&usize, &mut Window, &mut App) + 'static>;

/// Applies a `.tr-*` type style: family, nearest shipped face weight, size and line height.
pub trait TextStyled: Styled + Sized {
    fn text_style(self, style: TextStyle) -> Self {
        self.font_family(fonts::family(style.family))
            .font_weight(fonts::face_weight(&style))
            .text_size(style.size)
            .line_height(style.size * style.line_height)
    }
}

impl<T: Styled> TextStyled for T {}

/// Text with a type style, uppercased when the style says so.
pub fn label(style: TextStyle, content: impl Into<SharedString>) -> gpui_kit::Div {
    let content = content.into();
    let content: SharedString = if style.uppercase {
        content.to_uppercase().into()
    } else {
        content
    };
    div().text_style(style).child(content)
}

fn roles(cx: &App) -> crate::theme::Roles {
    ThinkrailTheme::global(cx).roles
}

/// Components hold closures and child elements, which have no `Debug`; name the type only.
macro_rules! opaque_debug {
    ($($ty:ident),* $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )*};
}

opaque_debug!(
    IconButton,
    PanelHeader,
    TreeRow,
    Tab,
    TabStrip,
    Segment,
    StatusDot,
    Breadcrumb,
    ActivityItem
);
