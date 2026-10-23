//! Refresh the displayed identity before rendering an explicit `/status` request.

use super::*;
use crate::status::StatusAccountDisplay;
use codex_app_server_protocol::Account;

impl App {
    pub(super) async fn show_account_status(
        &mut self,
        app_server: &mut AppServerSession,
        request_id: u64,
    ) {
        // account/read reloads externally changed credentials without refreshing tokens or models.
        match tokio::time::timeout(Duration::from_secs(/*secs*/ 2), app_server.read_account()).await
        {
            Ok(Ok(response)) => {
                let (display, plan, has_chatgpt_account) = match response.account {
                    Some(Account::Chatgpt { email, plan_type }) => (
                        Some(StatusAccountDisplay::ChatGpt {
                            email,
                            plan: Some(crate::status::plan_type_display_name(plan_type)),
                        }),
                        Some(plan_type),
                        true,
                    ),
                    Some(Account::ApiKey {}) => (Some(StatusAccountDisplay::ApiKey), None, false),
                    Some(Account::AmazonBedrock { .. }) | None => (None, None, false),
                };
                if self.chat_widget.status_account_display() != display.as_ref()
                    || self.chat_widget.current_plan_type() != plan
                    || self.chat_widget.has_chatgpt_account() != has_chatgpt_account
                {
                    self.last_thread_usage_status_cell = None;
                    self.pending_thread_usage_history_refresh = false;
                    self.chat_widget.update_account_state(
                        display,
                        plan,
                        has_chatgpt_account,
                        /*has_codex_backend_auth*/ has_chatgpt_account,
                    );
                }
            }
            Ok(Err(err)) => tracing::warn!(%err, "failed to refresh status account"),
            Err(_) => tracing::warn!("status account refresh timed out"),
        }
        let refresh_usage = self
            .chat_widget
            .config_ref()
            .model_provider
            .requires_openai_auth
            && self.chat_widget.has_chatgpt_account();
        self.chat_widget
            .add_status_output(refresh_usage, Some(request_id));
        if refresh_usage {
            self.refresh_rate_limits(
                app_server,
                RateLimitRefreshOrigin::StatusCommand { request_id },
            );
        }
    }
}
