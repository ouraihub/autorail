//! Seeding of the built-in provider presets (`thinkrail_core::PROVIDER_PRESETS`).
//!
//! A preset becomes an ordinary provider row with `preset_id` set: disabled, one endpoint, no
//! models, no keys. Seeding is idempotent per `preset_id`; once the row exists the user owns its
//! contents (name, address, models, keys) and seeding never touches it again.

use thinkrail_core::{Access, Endpoint, PROVIDER_PRESETS, ProviderInput, ProviderPreset, Result};
use tracing::warn;

use crate::{db::Conn, providers, validate};

fn input(preset: &ProviderPreset) -> ProviderInput {
    ProviderInput {
        name: preset.name.to_owned(),
        endpoints: vec![Endpoint { protocol: preset.protocol, base_url: preset.base_url.to_owned() }],
        default_protocol: preset.protocol,
        headers: Vec::new(),
        access: Access::Direct,
        enabled: false,
        models: Vec::new(),
    }
}

/// Inserts every preset that has no row yet; returns how many were inserted. A preset whose name
/// a user provider already took is skipped (logged), not renamed.
pub(crate) fn seed(conn: &Conn) -> Result<usize> {
    let mut seeded = 0;
    for preset in PROVIDER_PRESETS {
        if conn.row("SELECT 1 FROM providers WHERE preset_id = ?1", (preset.id,))?.is_some() {
            continue;
        }
        if conn.row("SELECT 1 FROM providers WHERE name = ?1", (preset.name,))?.is_some() {
            warn!(preset = preset.id, "preset skipped: a provider with its name exists");
            continue;
        }
        providers::insert(conn, &validate::provider(input(preset))?, Some(preset.id))?;
        seeded += 1;
    }
    Ok(seeded)
}
