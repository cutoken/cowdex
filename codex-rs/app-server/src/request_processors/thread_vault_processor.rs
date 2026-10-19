use std::sync::Arc;

use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ThreadVaultAttachParams;
use codex_app_server_protocol::ThreadVaultAttachResponse;
use codex_app_server_protocol::ThreadVaultDetachParams;
use codex_app_server_protocol::ThreadVaultDetachResponse;
use codex_app_server_protocol::ThreadVaultStatusParams;
use codex_app_server_protocol::ThreadVaultStatusResponse;
use codex_core::ThreadManager;
use codex_protocol::ThreadId;
use codex_vault_extension::VaultService;

use crate::error_code::invalid_request;

pub(crate) struct ThreadVaultRequestProcessor {
    pub(crate) thread_manager: Arc<ThreadManager>,
    pub(crate) service: Arc<VaultService>,
}

impl ThreadVaultRequestProcessor {
    async fn loaded_thread(&self, id: &str) -> Result<ThreadId, JSONRPCErrorError> {
        let id = ThreadId::from_string(id).map_err(|_| invalid_request("invalid thread id"))?;
        self.thread_manager
            .get_thread(id)
            .await
            .map_err(|_| invalid_request("vault requires a loaded thread"))?;
        Ok(id)
    }

    pub(crate) async fn attach(
        &self,
        params: ThreadVaultAttachParams,
    ) -> Result<ThreadVaultAttachResponse, JSONRPCErrorError> {
        let id = self.loaded_thread(&params.thread_id).await?;
        // Empty profile/scope select the human-approved defaults from the vault page.
        if params.scope.len() > 8
            || params.scope.iter().any(|alias| {
                alias.is_empty()
                    || alias.len() > 32
                    || !alias
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            })
        {
            return Err(invalid_request(
                "scope must contain at most 8 aliases of 1–32 ASCII letters, digits, _, or -",
            ));
        }
        let status = self
            .service
            .attach(id, params.profile.into(), params.scope)
            .await
            .map_err(invalid_request)?;
        Ok(ThreadVaultAttachResponse {
            aliases: status.aliases,
            expires_at: status.expires_at,
        })
    }

    pub(crate) async fn status(
        &self,
        params: ThreadVaultStatusParams,
    ) -> Result<ThreadVaultStatusResponse, JSONRPCErrorError> {
        let id = self.loaded_thread(&params.thread_id).await?;
        let status = self.service.status(id).await.map_err(invalid_request)?;
        Ok(ThreadVaultStatusResponse {
            aliases: status.aliases,
            expires_at: status.expires_at,
        })
    }

    pub(crate) async fn detach(
        &self,
        params: ThreadVaultDetachParams,
    ) -> Result<ThreadVaultDetachResponse, JSONRPCErrorError> {
        let id = self.loaded_thread(&params.thread_id).await?;
        let status = self.service.detach(id).await.map_err(invalid_request)?;
        Ok(ThreadVaultDetachResponse {
            aliases: status.aliases,
            expires_at: status.expires_at,
        })
    }
}
