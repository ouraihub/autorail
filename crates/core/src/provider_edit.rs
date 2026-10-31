//! Pure edits of a provider's lines (endpoints) that keep models and the default protocol
//! consistent with the remaining lines (agent-settings 04 §7.6). Validation stays in the store.

use crate::{Endpoint, Protocol, ProviderInput};

/// Every protocol, in the order lines are offered.
pub const LINE_PROTOCOLS: [Protocol; 3] = [Protocol::OpenaiChat, Protocol::AnthropicMessages, Protocol::OpenaiResponses];

/// Protocols the provider has no line for yet.
pub fn free_protocols(input: &ProviderInput) -> Vec<Protocol> {
    LINE_PROTOCOLS.into_iter().filter(|p| !input.endpoints.iter().any(|e| e.protocol == *p)).collect()
}

/// Adds or replaces the line for `protocol`.
pub fn set_line(input: &mut ProviderInput, protocol: Protocol, base_url: String) {
    match input.endpoints.iter_mut().find(|endpoint| endpoint.protocol == protocol) {
        Some(endpoint) => endpoint.base_url = base_url,
        None => input.endpoints.push(Endpoint { protocol, base_url }),
    }
}

/// Moves the line `from` to the unused protocol `to`: models and the default follow it.
/// Returns false (and changes nothing) when `from` is missing or `to` is taken.
pub fn retarget_line(input: &mut ProviderInput, from: Protocol, to: Protocol) -> bool {
    if from == to {
        return input.endpoints.iter().any(|e| e.protocol == from);
    }
    if input.endpoints.iter().any(|e| e.protocol == to) {
        return false;
    }
    let Some(endpoint) = input.endpoints.iter_mut().find(|e| e.protocol == from) else { return false };
    endpoint.protocol = to;
    for model in &mut input.models {
        for protocol in &mut model.protocols {
            if *protocol == from {
                *protocol = to;
            }
        }
    }
    if input.default_protocol == from {
        input.default_protocol = to;
    }
    true
}

/// Removes the line for `protocol`. Models lose it and are dropped when no line is left for
/// them; the default moves to the first remaining line. Refuses (returns false) to remove the
/// last line, because a provider needs one address.
pub fn remove_line(input: &mut ProviderInput, protocol: Protocol) -> bool {
    if input.endpoints.len() <= 1 || !input.endpoints.iter().any(|e| e.protocol == protocol) {
        return false;
    }
    input.endpoints.retain(|endpoint| endpoint.protocol != protocol);
    input.models.retain_mut(|model| {
        model.protocols.retain(|item| *item != protocol);
        !model.protocols.is_empty()
    });
    if input.default_protocol == protocol {
        input.default_protocol = input.endpoints[0].protocol;
    }
    true
}

/// Models served on the line `protocol`, in list order, with their index in `input.models`.
pub fn line_models(input: &ProviderInput, protocol: Protocol) -> impl Iterator<Item = (usize, &crate::ProviderModel)> {
    input.models.iter().enumerate().filter(move |(_, model)| model.protocols.contains(&protocol))
}
