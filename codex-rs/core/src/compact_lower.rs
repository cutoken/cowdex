//! Request-only compaction downshifts leave the working turn and its context limits intact.

use std::borrow::Cow;

use crate::Prompt;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_otel::SessionTelemetry;
use codex_protocol::models::ConfigurationReasoning;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort;

pub(crate) struct CompactionRequest<'a> {
    pub(crate) model_info: Cow<'a, ModelInfo>,
    pub(crate) effort: Option<ReasoningEffort>,
    pub(crate) prompt: Cow<'a, Prompt>,
    pub(crate) telemetry: Cow<'a, SessionTelemetry>,
}

impl<'a> CompactionRequest<'a> {
    pub(crate) async fn new(sess: &Session, turn: &'a TurnContext, prompt: &'a Prompt) -> Self {
        let mut request = Self {
            model_info: Cow::Borrowed(&turn.model_info),
            effort: turn.reasoning_effort.clone(),
            prompt: Cow::Borrowed(prompt),
            telemetry: Cow::Borrowed(&turn.session_telemetry),
        };
        if !turn.config.compact_with_lower {
            return request;
        }
        let model = turn.model_info.slug.as_str();
        let effort = sess
            .services
            .model_client
            .build_reasoning(
                &turn.model_info,
                turn.reasoning_effort.clone(),
                turn.reasoning_summary,
            )
            .effort;
        if !(model.starts_with("gpt-6-astra")
            || model.ends_with(".gpt-6-astra")
            || (model.starts_with("gpt-5.6-sol") || model.ends_with(".gpt-5.6-sol"))
                && matches!(
                    effort,
                    Some(ReasoningEffort::High | ReasoningEffort::XHigh | ReasoningEffort::Max)
                ))
        {
            return request;
        }
        request.model_info = Cow::Owned(
            sess.services
                .models_manager
                .get_model_info("gpt-5.6-sol", &turn.config.to_models_manager_config())
                .await,
        );
        request.effort = Some(ReasoningEffort::Medium);
        request.telemetry = Cow::Owned(
            turn.session_telemetry
                .clone()
                .with_model("gpt-5.6-sol", &request.model_info.slug),
        );
        // A surviving effort override can supersede the request-level effort. Add a
        // transient override at the end, preserving the cached history prefix and
        // keeping a terminal compaction trigger last.
        // This prompt is never written back into the working conversation history.
        if request.model_info.use_responses_lite
            && prompt
                .input
                .iter()
                .rev()
                .find_map(|item| match item {
                    ResponseItem::ConfigurationUpdate { reasoning } => Some(&reasoning.effort),
                    _ => None,
                })
                .is_some_and(|effort| effort != &ReasoningEffort::Medium)
        {
            let index = match prompt.input.last() {
                Some(ResponseItem::CompactionTrigger { .. }) => prompt.input.len() - 1,
                _ => prompt.input.len(),
            };
            request.prompt.to_mut().input.insert(
                index,
                ResponseItem::ConfigurationUpdate {
                    reasoning: ConfigurationReasoning {
                        effort: ReasoningEffort::Medium,
                    },
                },
            );
        }
        request
    }
}
