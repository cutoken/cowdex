//! Apply backend-authorized Reserve at turn boundaries without changing global model defaults.

use super::App;
use super::luna_reserve_return::ReserveReturnModel;
use crate::app_command::AppCommand;
use crate::app_server_session::AppServerSession;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::GetAccountRateLimitsParams;
use codex_app_server_protocol::GetAccountRateLimitsResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadSettingsUpdateParams;
use codex_protocol::ThreadId;
use codex_protocol::openai_models::ModelPreset;
use color_eyre::eyre::Result;
use std::time::Duration;

const RESERVE_MODEL: &str = "gpt-reserve";

impl App {
    pub(super) async fn prepare_luna_reserve_turn(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
        op: &mut AppCommand,
    ) -> Result<()> {
        if !matches!(op, AppCommand::UserTurn { .. })
            || self.active_thread_id != Some(thread_id)
            || self.chat_widget.config_ref().model_provider_id != "openai"
            || self.active_turn_id_for_thread(thread_id).await.is_some()
        {
            return Ok(());
        }
        let models = self.model_catalog.try_list_models()?;
        // Reserve is a backend catalog alias, not an arbitrary manually selected model.
        if !models.iter().any(|preset| preset.model == RESERVE_MODEL) {
            return Ok(());
        }
        let request_handle = app_server.request_handle();
        let request = request_handle.request_typed::<GetAccountRateLimitsResponse>(
            ClientRequest::GetAccountRateLimits {
                request_id: RequestId::String(format!("reserve-{}", uuid::Uuid::new_v4())),
                params: Some(GetAccountRateLimitsParams {
                    supports_luna_reserve: true,
                    exclude_reset_credit_details: true,
                }),
            },
        );
        let Ok(Ok(usage)) = tokio::time::timeout(Duration::from_secs(5), request).await else {
            return Ok(());
        };
        let home = self.chat_widget.config_ref().codex_home.clone();
        let previous = ReserveReturnModel::load(home.as_path(), thread_id);
        let AppCommand::UserTurn {
            model,
            effort,
            service_tier,
            collaboration_mode,
            ..
        } = op
        else {
            return Ok(());
        };
        let Some(target) = reserve_target(&usage, model, previous.as_ref(), &models) else {
            return Ok(());
        };
        let entering = target.model == RESERVE_MODEL;
        let desired_effort = if entering {
            effort.clone()
        } else {
            previous
                .as_ref()
                .and_then(|previous| previous.effort.clone())
        };
        let target_effort = desired_effort
            .filter(|effort| {
                target
                    .supported_reasoning_efforts
                    .iter()
                    .any(|supported| &supported.effort == effort)
            })
            .unwrap_or_else(|| target.default_reasoning_effort.clone());
        if entering {
            let Some(account_id) = usage.account_id.clone() else {
                return Ok(());
            };
            let previous = ReserveReturnModel {
                account_id,
                model: model.clone(),
                effort: effort.clone(),
            };
            previous.save(home.as_path(), thread_id)?;
        }
        let mode = self.chat_widget.current_collaboration_mode().with_updates(
            Some(target.model.clone()),
            Some(Some(target_effort.clone())),
            /*developer_instructions*/ None,
        );
        if !app_server
            .thread_settings_update(ThreadSettingsUpdateParams {
                thread_id: thread_id.to_string(),
                model: Some(target.model.clone()),
                effort: Some(target_effort.clone()),
                service_tier: Some(None),
                collaboration_mode: Some(mode.clone()),
                ..Default::default()
            })
            .await?
        {
            return Ok(());
        }
        *model = target.model.clone();
        *effort = Some(target_effort.clone());
        *service_tier = Some(None);
        *collaboration_mode = Some(mode);
        self.chat_widget.set_model(&target.model);
        self.chat_widget.set_reasoning_effort(Some(target_effort));
        if !entering {
            ReserveReturnModel::clear(home.as_path(), thread_id);
        }
        self.chat_widget
            .add_info_message(reserve_notice(target), /*hint*/ None);
        Ok(())
    }
}

fn reserve_notice(target: &ModelPreset) -> String {
    if target.model == RESERVE_MODEL {
        "Ordinary usage is exhausted. Continuing with Luna Reserve.".to_string()
    } else {
        format!(
            "Ordinary usage is available again. Returning to {}.",
            target.display_name
        )
    }
}

fn reserve_target<'a>(
    usage: &GetAccountRateLimitsResponse,
    current_model: &str,
    previous: Option<&ReserveReturnModel>,
    models: &'a [ModelPreset],
) -> Option<&'a ModelPreset> {
    // The app server validates these identities against the active login after the read.
    let account_id = usage.account_id.as_deref()?;
    usage.user_id.as_ref()?;
    if current_model == RESERVE_MODEL {
        let has_credits = usage
            .rate_limits
            .credits
            .as_ref()
            .is_some_and(|credits| credits.unlimited || credits.has_credits);
        if usage.ordinary_usage_allowed.is_some()
            && (usage.ordinary_usage_allowed == Some(true) || has_credits)
            && usage.rate_limit_upsell.is_none()
            && usage.rate_limits.spend_control_reached != Some(true)
            && usage.rate_limits.rate_limit_reached_type.is_none()
        {
            let previous = previous.filter(|previous| previous.account_id == account_id)?;
            return models
                .iter()
                .find(|model| model.show_in_picker && model.model == previous.model);
        }
        return None;
    }
    let banner = usage.rate_limit_upsell.as_ref()?;
    if banner.get("banner_type")?.as_str()? != "luna_reserve" {
        return None;
    }
    if let Some(blocked) = banner
        .get("blocked_model_slug")
        .and_then(serde_json::Value::as_str)
        && blocked != current_model
    {
        return None;
    }
    models.iter().find(|model| model.model == RESERVE_MODEL)
}

#[cfg(test)]
#[path = "luna_reserve_tests.rs"]
mod tests;
