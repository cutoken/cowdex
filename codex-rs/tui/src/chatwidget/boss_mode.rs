//! Live worker defaults, shown only after the server accepts the settings.

use super::*;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::protocol::BossModeSettings;

const USAGE: &str = "Use /boss <model> <effort>, /boss on, or /boss off. Models: astra, sol, terra, luna, or a full model ID. Applies to new workers.";

impl ChatWidget {
    pub(super) fn dispatch_boss_command(&mut self, args: &str) {
        let models = self.model_catalog.try_list_models().unwrap_or_default();
        match parse_boss_command(args, self.config.multi_agent_v2.boss_mode.as_ref(), &models) {
            Ok(Some(settings)) => {
                let Some(thread_id) = self.thread_id else {
                    self.add_error_message(
                        "The session is starting. Try /boss again shortly.".to_string(),
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
                    None => "Boss off · default: sol medium".to_string(),
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
        model: "gpt-5.6-sol".to_string(),
        reasoning_effort: ReasoningEffortConfig::Medium,
    });
    match words.as_slice() {
        [] => return Ok(None),
        ["off"] => {
            settings.enabled = false;
            return Ok(Some(settings));
        }
        ["on"] => settings.enabled = true,
        [model, effort] => {
            settings.model = match *model {
                "astra" => "gpt-6-astra",
                "sol" => "gpt-5.6-sol",
                "terra" => "gpt-5.6-terra",
                "luna" => "gpt-5.6-luna",
                model => model,
            }
            .to_string();
            settings.reasoning_effort = effort
                .parse()
                .map_err(|_| format!("Unknown reasoning effort '{effort}'. {USAGE}"))?;
            settings.enabled = true;
        }
        _ => return Err(USAGE.to_string()),
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
        "gpt-5.6-sol" => "sol",
        "gpt-5.6-terra" => "terra",
        "gpt-5.6-luna" => "luna",
        model => model,
    };
    format!("Boss {model} {}", settings.reasoning_effort)
}
