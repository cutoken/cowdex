use super::*;
use pretty_assertions::assert_eq;

#[test]
fn selectors_preserve_provider_model_ids_and_separate_effort() {
    for (input, provider, model, effort) in [
        ("sol:high", None, "gpt-6.1-sol", Some(ReasoningEffort::High)),
        (":low", None, "gpt-6.1-sol", Some(ReasoningEffort::Low)),
        ("zai:glm-5.3", Some("zai"), "glm-5.3", None),
        (
            "zai:glm-5.3:max",
            Some("zai"),
            "glm-5.3",
            Some(ReasoningEffort::Max),
        ),
        (
            "openrouter:deepseek/model:free",
            Some("openrouter"),
            "deepseek/model:free",
            None,
        ),
        (
            "openrouter:deepseek/model:free:high",
            Some("openrouter"),
            "deepseek/model:free",
            Some(ReasoningEffort::High),
        ),
        ("openrouter:sol", Some("openrouter"), "sol", None),
    ] {
        let parsed = WorkerModel::parse(input).unwrap();
        assert_eq!(
            parsed,
            WorkerModel {
                provider,
                model,
                effort
            }
        );
        assert_eq!(
            parsed.qualified_model(),
            provider.map_or_else(
                || model.to_string(),
                |provider| format!("{provider}:{model}")
            )
        );
    }
    for invalid in [
        "zai:",
        "zai::high",
        "openrouter:",
        "openrouter::max",
        "sol:extreme",
        "sol:high:low",
        ":",
    ] {
        assert!(WorkerModel::parse(invalid).is_err(), "{invalid}");
    }
}
