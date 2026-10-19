//! Opt-in, harness-authored effort updates append without rewriting surviving history.

use super::session::Session;
use super::step_context::StepContext;
use codex_features::Feature;
use codex_history::CodexHarnessMetadata;
use codex_history::ResponseItemEnvelope;
use codex_protocol::models::ConfigurationReasoning;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;

impl Session {
    pub(crate) async fn record_reasoning_effort_override(&self, step: &StepContext) {
        if !self.enabled(Feature::ReasoningEffortOverride)
            || !step.model_info.use_responses_lite
            || !self.provider().await.is_openai()
        {
            return;
        }
        // Use the request resolver so Cowdex's Ultra overrides and Astra caps agree.
        let Some(effort) = self
            .services
            .model_client
            .build_reasoning(
                &step.model_info,
                step.reasoning_effort.clone(),
                step.reasoning_summary,
            )
            .effort
        else {
            return;
        };
        // Keep durable updates bounded to known backend modes.
        if matches!(&effort, ReasoningEffort::Custom(_)) {
            return;
        }
        let unchanged = {
            let state = self.state.lock().await;
            let established = state
                .history
                .annotated_items()
                .iter()
                .rev()
                .find_map(|envelope| {
                    if !envelope
                        .metadata
                        .as_ref()
                        .is_some_and(|metadata| metadata.harness_authored_configuration)
                    {
                        return None;
                    }
                    match &envelope.item {
                        ResponseItem::ConfigurationUpdate { reasoning } => Some(&reasoning.effort),
                        _ => None,
                    }
                });
            established == Some(&effort)
        };
        if unchanged {
            return;
        }
        self.record_annotated_conversation_items(
            step.turn.as_ref(),
            vec![ResponseItemEnvelope {
                item: ResponseItem::ConfigurationUpdate {
                    reasoning: ConfigurationReasoning { effort },
                },
                metadata: Some(CodexHarnessMetadata {
                    harness_authored_configuration: true,
                    ..Default::default()
                }),
            }],
        )
        .await;
    }
}
