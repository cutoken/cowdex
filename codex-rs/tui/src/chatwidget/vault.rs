use super::ChatWidget;
use crate::app_event::AppEvent;
use crate::vault_command::VAULT_USAGE;
use crate::vault_command::VaultCommand;

impl ChatWidget {
    pub(super) fn handle_vault_command(&mut self, args: &str) {
        if args.is_empty() {
            self.add_info_message(VAULT_USAGE.to_string(), /*hint*/ None);
            return;
        }
        let command = match VaultCommand::parse(args) {
            Ok(command) => command,
            Err(message) => {
                self.add_error_message(message.to_string());
                return;
            }
        };
        let Some(thread_id) = self.thread_id else {
            self.add_error_message("Start a session before attaching a vault.".to_string());
            return;
        };
        self.app_event_tx
            .send(AppEvent::VaultCommand { thread_id, command });
    }
}
