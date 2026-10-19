use super::AppServerSession;
use crate::vault_command::VaultCommand;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::ThreadVaultAttachParams;
use codex_app_server_protocol::ThreadVaultAttachResponse;
use codex_app_server_protocol::ThreadVaultDetachParams;
use codex_app_server_protocol::ThreadVaultDetachResponse;
use codex_app_server_protocol::ThreadVaultStatusParams;
use codex_app_server_protocol::ThreadVaultStatusResponse;
use codex_protocol::ThreadId;
use color_eyre::eyre::Result;

impl AppServerSession {
    pub(crate) async fn vault_command(
        &mut self,
        thread_id: ThreadId,
        command: VaultCommand,
    ) -> Result<ThreadVaultStatusResponse> {
        let request_id = self.next_request_id();
        let thread_id = thread_id.to_string();
        match command {
            VaultCommand::Attach { profile, scope } => {
                let response: ThreadVaultAttachResponse = self
                    .client
                    .request_typed(ClientRequest::ThreadVaultAttach {
                        request_id,
                        params: ThreadVaultAttachParams {
                            thread_id,
                            profile,
                            scope,
                        },
                    })
                    .await?;
                Ok(ThreadVaultStatusResponse {
                    aliases: response.aliases,
                    expires_at: response.expires_at,
                })
            }
            VaultCommand::Status => Ok(self
                .client
                .request_typed(ClientRequest::ThreadVaultStatus {
                    request_id,
                    params: ThreadVaultStatusParams { thread_id },
                })
                .await?),
            VaultCommand::Detach => {
                let response: ThreadVaultDetachResponse = self
                    .client
                    .request_typed(ClientRequest::ThreadVaultDetach {
                        request_id,
                        params: ThreadVaultDetachParams { thread_id },
                    })
                    .await?;
                Ok(ThreadVaultStatusResponse {
                    aliases: response.aliases,
                    expires_at: response.expires_at,
                })
            }
        }
    }
}
