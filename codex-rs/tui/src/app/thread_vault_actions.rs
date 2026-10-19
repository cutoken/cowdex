use super::App;
use crate::app_server_session::AppServerSession;
use crate::vault_command::VaultCommand;
use crate::vault_command::approval_text;
use crate::vault_command::status_text;
use codex_app_server_client::TypedRequestError;
use codex_protocol::ThreadId;

impl App {
    pub(super) async fn run_vault_command(
        &mut self,
        app_server: &mut AppServerSession,
        thread_id: ThreadId,
        command: VaultCommand,
    ) {
        let result = app_server.vault_command(thread_id, command).await;
        if self.current_displayed_thread_id() != Some(thread_id) {
            return;
        }
        match result {
            Ok(status) => self
                .chat_widget
                .add_info_message(status_text(&status), /*hint*/ None),
            Err(error) => {
                if let Some(message) = error
                    .downcast_ref::<TypedRequestError>()
                    .and_then(approval_text)
                {
                    self.chat_widget.add_info_message(message, /*hint*/ None);
                } else {
                    self.chat_widget.add_error_message("Vault request failed. Open the vault page, unlock it, and select credentials for Cowdex. Then run /vault attach again.".to_string());
                }
            }
        }
    }
}
