//! Live worker defaults, shown only after the server accepts the settings.

use super::*;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::protocol::BossModeSettings;
use codex_protocol::worker_model::WorkerModel;

const USAGE: &str = "Use /worker <model> [effort], /worker on, or /worker off. Models: sol, astra, luna, a full ID, or provider:model. Applies to new workers; /model changes the lead.";

impl ChatWidget {
    pub(super) fn dispatch_boss_command(&mut self, args: &str) {
        let models = self.model_catalog.try_list_models().unwrap_or_default();
        match parse_boss_command(args, self.config.multi_agent_v2.boss_mode.as_ref(), &models) {
            Ok(Some(settings)) => {
                let Some(thread_id) = self.thread_id else {
                    self.add_error_message(
                        "The session is starting. Try /worker again shortly.".to_string(),
                    );
                    return;
                };
                self.app_event_tx.send(AppEvent::UpdateBossMode {
                    thread_id,
                    settings,
                });
            }
            Ok(None) => {
                let current = self.config.multi_agent_v2.boss_mode.as_ref();
                let status = match current {
                    Some(settings) if settings.enabled => {
                        format!("Boss on · {} {}", settings.model, settings.reasoning_effort)
                    }
                    Some(settings) => format!(
                        "Boss off · saved: {} {}",
                        settings.model, settings.reasoning_effort
                    ),
                    None => "Boss off · workers follow the lead".to_string(),
                };
                self.add_info_message(format!("{status}\n{USAGE}"), /*hint*/ None);
            }
            Err(error) => self.add_error_message(error),
        }
    }
}

pub(super) fn parse_boss_command(
    args: &str,
    current: Option<&BossModeSettings>,
    models: &[ModelPreset],
) -> Result<Option<BossModeSettings>, String> {
    let words: Vec<_> = args.split_whitespace().collect();
    let mut settings = current.cloned().unwrap_or(BossModeSettings {
        enabled: false,
        model: "gpt-6.1-sol".to_string(),
        reasoning_effort: ReasoningEffortConfig::Medium,
    });
    match words.as_slice() {
        [] => return Ok(None),
        ["off"] => {
            settings.enabled = false;
            return Ok(Some(settings));
        }
        ["on"] => settings.enabled = true,
        [model] | [model, _] => {
            let model = match *model {
                "terra" => "gpt-5.6-terra",
                model => model,
            };
            let selected = WorkerModel::parse(model)?;
            settings.model = selected.qualified_model();
            settings.reasoning_effort = match words.get(1) {
                Some(effort) => effort
                    .parse()
                    .map_err(|_| format!("Unknown reasoning effort '{effort}'. {USAGE}"))?,
                None => selected.effort.unwrap_or(ReasoningEffortConfig::Medium),
            };
            settings.enabled = true;
        }
        _ => return Err(USAGE.to_string()),
    }
    if WorkerModel::parse(&settings.model)?.provider.is_some() {
        return if matches!(
            settings.reasoning_effort,
            ReasoningEffortConfig::Low
                | ReasoningEffortConfig::Medium
                | ReasoningEffortConfig::High
                | ReasoningEffortConfig::XHigh
                | ReasoningEffortConfig::Max
        ) {
            Ok(Some(settings))
        } else {
            Err("Worker effort must be low, medium, high, xhigh, or max.".to_string())
        };
    }
    let model = models
        .iter()
        .find(|model| model.model == settings.model)
        .ok_or_else(|| format!("Model '{}' is not available. {USAGE}", settings.model))?;
    if !model
        .supported_reasoning_efforts
        .iter()
        .any(|option| option.effort == settings.reasoning_effort)
    {
        let supported = model
            .supported_reasoning_efforts
            .iter()
            .map(|option| option.effort.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("{} supports: {supported}.", settings.model));
    }
    Ok(Some(settings))
}

pub(super) fn boss_status_label(settings: &BossModeSettings) -> String {
    let model = match settings.model.as_str() {
        "gpt-6-astra" => "astra",
        "gpt-6.1-sol" => "sol",
        "gpt-5.6-terra" => "terra",
        "gpt-6-luna" => "luna",
        model => model,
    };
    format!("Boss {model} {}", settings.reasoning_effort)
}
