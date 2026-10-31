//! The original's three OFL families, bundled as TTF (DirectWrite cannot load the web build's
//! woff2) and registered once at startup. Licenses ship next to the files in `assets/fonts`.

use std::borrow::Cow;

use gpui_kit::{App, FontWeight};

use crate::metrics::{Family, TextStyle};

pub const INTERFACE: &str = "Geist";
pub const CODE: &str = "JetBrains Mono";
pub const BRAND: &str = "Orbitron";

const FILES: [&[u8]; 7] = [
    include_bytes!("../assets/fonts/Geist-Light.ttf"),
    include_bytes!("../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf"),
    include_bytes!("../assets/fonts/Orbitron-Variable.ttf"),
];

/// Registers the bundled faces with the platform text system.
pub fn register(cx: &App) -> anyhow::Result<()> {
    cx.text_system().add_fonts(FILES.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
}

pub fn family(family: Family) -> &'static str {
    match family {
        Family::Interface => INTERFACE,
        Family::Code => CODE,
        Family::Brand => BRAND,
    }
}

/// The bundled face closest to a style's weight. The original's variable fonts render 370 / 470
/// exactly; the static faces round to the nearest shipped weight (370 → 400, 470 → 500).
pub fn face_weight(style: &TextStyle) -> FontWeight {
    let shipped: &[f32] = match style.family {
        Family::Interface => &[300., 400., 500., 600.],
        Family::Code => &[400., 500.],
        Family::Brand => &[400.],
    };
    let target = style.weight.0;
    let nearest = shipped
        .iter()
        .copied()
        .min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()).then(b.total_cmp(a)))
        .unwrap_or(target);
    FontWeight(nearest)
}
