use super::*;
use pretty_assertions::assert_eq;

#[test]
fn astra_ultra_display_preserves_higher_efforts() {
    assert_eq!(
        reasoning_effort_label(
            &ReasoningEffort::Ultra,
            &ReasoningEffort::Max,
            "gpt-6-astra",
        ),
        "ultra(max)"
    );
    assert_eq!(
        reasoning_effort_label(
            &ReasoningEffort::Ultra,
            &ReasoningEffort::XHigh,
            "global.openai.gpt-6-astra",
        ),
        "ultra(xhigh)"
    );
    assert_eq!(
        reasoning_effort_label(
            &ReasoningEffort::Ultra,
            &ReasoningEffort::Max,
            "gpt-5.6-sol",
        ),
        "ultra(max)"
    );
}

#[test]
fn astra_effort_labels_snapshot() {
    let labels: Vec<_> = [
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::XHigh,
        ReasoningEffort::Max,
        ReasoningEffort::Ultra,
    ]
    .iter()
    .map(|effort| {
        format!(
            "{effort}: {}",
            reasoning_effort_label(effort, &ReasoningEffort::High, "gpt-6-astra")
        )
    })
    .collect();
    insta::assert_snapshot!(labels.join("\n"), @"
    low: low
    medium: medium
    high: high
    xhigh: xhigh
    max: max
    ultra: ultra(high)
    ");
}
