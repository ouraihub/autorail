//! What the settings page needs from routing (agent-settings 08 §4.3–4.4): every routable model
//! regardless of any engine's 编码搭档 choice, why a saved model cannot be routed, and which
//! engines a choice keeps away from these settings.

use thinkrail_core::{
    BackendKind, EngineNote, EngineProvider, ModelChoice, ModelPrefs, ModelRef, ModelStatus,
    Provider, Result,
};

use super::{context_override, plan, project_model, route_for, usable};

/// Routes without a 编码搭档 choice. Every engine speaks every protocol, so the backend only
/// names the call; no choice never fails.
fn unrestricted<'a>(
    providers: &'a [Provider],
    prefs: &ModelPrefs,
) -> Result<Vec<super::plan::PlannedRoute<'a>>> {
    plan(providers, BackendKind::Pi, None, &prefs.context_overrides)
}

pub(super) fn catalog(providers: &[Provider], prefs: &ModelPrefs) -> Result<Vec<ModelChoice>> {
    let routes = unrestricted(providers, prefs)?;
    let mut out = Vec::new();
    for provider in providers.iter().filter(|p| p.input.enabled) {
        for model in provider.input.models.iter().filter(|m| usable(m)) {
            let Some(route) = route_for(&routes, provider, model) else {
                continue;
            };
            out.push(ModelChoice {
                provider: provider.id.clone(),
                provider_name: provider.input.name.clone(),
                route: route.to_owned(),
                model: project_model(
                    model,
                    context_override(&prefs.context_overrides, provider, model),
                ),
            });
        }
    }
    Ok(out)
}

pub(super) fn status(providers: &[Provider], wanted: &ModelRef) -> Result<ModelStatus> {
    let Some(provider) = providers.iter().find(|p| p.id == wanted.provider) else {
        return Ok(ModelStatus::ProviderMissing);
    };
    if !provider.input.enabled {
        return Ok(ModelStatus::ProviderDisabled {
            provider: provider.input.name.clone(),
        });
    }
    let Some(model) = provider
        .input
        .models
        .iter()
        .find(|m| m.model_id == wanted.model_id)
    else {
        return Ok(ModelStatus::ModelMissing);
    };
    Ok(if usable(model) {
        ModelStatus::Available
    } else {
        ModelStatus::ModelUnusable {
            model: model.name.clone(),
        }
    })
}

pub(super) fn notes(providers: &[Provider], prefs: &ModelPrefs) -> Vec<(BackendKind, EngineNote)> {
    BackendKind::ALL
        .into_iter()
        .filter_map(|backend| {
            let note = match prefs.engine_provider(backend)? {
                EngineProvider::Gateway => return None,
                EngineProvider::Official => EngineNote::Official,
                EngineProvider::Provider(id) => {
                    match providers.iter().find(|p| &p.id == id && p.input.enabled) {
                        Some(provider) => EngineNote::OnlyProvider {
                            provider: provider.input.name.clone(),
                        },
                        None => EngineNote::ProviderUnavailable,
                    }
                }
            };
            Some((backend, note))
        })
        .collect()
}
