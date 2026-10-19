use codex_protocol::openai_models::ReasoningEffort;

pub(crate) fn reasoning_effort_label(
    effort: &ReasoningEffort,
    ultra_reasoning_effort: &ReasoningEffort,
    model: &str,
) -> String {
    let resolved = if effort == &ReasoningEffort::Ultra {
        ultra_reasoning_effort
    } else {
        effort
    };
    let resolved = if (model.starts_with("gpt-6-astra") || model.ends_with(".gpt-6-astra"))
        && !matches!(
            resolved,
            ReasoningEffort::Low
                | ReasoningEffort::Medium
                | ReasoningEffort::High
                | ReasoningEffort::XHigh
                | ReasoningEffort::Max
        ) {
        &ReasoningEffort::Medium
    } else {
        resolved
    };
    if effort == &ReasoningEffort::Ultra {
        format!("ultra({resolved})")
    } else {
        resolved.to_string()
    }
}

#[cfg(test)]
#[path = "reasoning_display_tests.rs"]
mod tests;
