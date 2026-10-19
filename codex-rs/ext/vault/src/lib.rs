//! Session-scoped access to an external credential broker. No provider keys enter Cowdex.

mod extension;
mod transport;

pub use extension::install;

use codex_protocol::ThreadId;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::Mutex;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct VaultStatus {
    pub aliases: Vec<String>,
    pub expires_at: Option<i64>,
}

// Intentionally no Debug or Serialize: capabilities must never enter logs or history.
struct Grant {
    socket: PathBuf,
    session: String,
    status: VaultStatus,
}

// Claim capabilities stay in memory and must never enter logs or history.
struct Pending {
    socket: PathBuf,
    approval: Approval,
}

#[derive(Deserialize)]
struct Approval {
    claim_token: String,
    approval_code: String,
    aliases: Vec<String>,
    expires_at: i64,
}

#[derive(Default)]
struct Entry {
    parent: Option<ThreadId>,
    grant: Option<Grant>,
    pending: Option<Pending>,
}

/// Owns live thread authority; lifecycle removal and explicit detach fail closed.
#[derive(Default)]
pub struct VaultService {
    entries: Mutex<HashMap<ThreadId, Entry>>,
}

// Broker exchanges are time-bounded and never call back into this service. Keep
// authority changes serialized with requests so detach cannot race credential use.
#[expect(
    clippy::await_holding_invalid_type,
    reason = "Serialize broker requests with grant attachment and revocation"
)]
impl VaultService {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn attach(
        &self,
        thread_id: ThreadId,
        profile: PathBuf,
        mut scope: Vec<String>,
    ) -> Result<VaultStatus, String> {
        let profile = transport::read_profile(profile).await?;
        if scope.is_empty() {
            scope = profile.scope;
        }
        validate_aliases(&scope)?;
        scope.sort();
        if scope.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("Vault scope must not repeat aliases".into());
        }
        let mut entries = self.entries.lock().await;
        let entry = entries
            .get_mut(&thread_id)
            .ok_or("Vault thread is not active")?;
        let now = unix_time()?;
        let pending = entry.pending.take();
        let claiming = pending.as_ref().is_some_and(|pending| {
            pending.socket == profile.socket
                && pending.approval.aliases == scope
                && pending.approval.expires_at > now
        });
        let request = if let Some(pending) = pending.as_ref().filter(|_| claiming) {
            json!({"op": "claim", "claim_token": pending.approval.claim_token})
        } else {
            if let Some(pending) = pending.as_ref() {
                let _ = transport::exchange(
                    &pending.socket,
                    json!({"op": "cancel", "claim_token": pending.approval.claim_token}),
                )
                .await;
            }
            json!({"op": "attach", "ticket": profile.ticket, "scope": scope})
        };
        let result = transport::exchange(&profile.socket, request).await?;
        let now = unix_time()?;
        if result.get("claim_token").is_some() {
            let approval: Approval =
                serde_json::from_value(result).map_err(|_| "Invalid vault approval response")?;
            if approval.aliases != scope
                || approval.claim_token.is_empty()
                || approval.claim_token.len() > 128
                || !approval
                    .claim_token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
                || approval.approval_code.len() != 12
                || !approval
                    .approval_code
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
                || approval.expires_at <= now
                || approval.expires_at > now.saturating_add(600)
                || claiming
                    && pending.as_ref().is_some_and(|pending| {
                        pending.approval.claim_token != approval.claim_token
                            || pending.approval.approval_code != approval.approval_code
                            || pending.approval.expires_at != approval.expires_at
                    })
            {
                return Err("Invalid vault approval response".into());
            }
            let code = &approval.approval_code;
            let message = format!(
                "Vault approval required: {code}. Approve the matching request in the vault page, then repeat the same /vault attach command, including its options."
            );
            entry.pending = Some(Pending {
                socket: profile.socket,
                approval,
            });
            return Err(message);
        }
        #[derive(Deserialize)]
        struct Attached {
            session: String,
            aliases: Vec<String>,
            expires_at: i64,
        }
        let attached: Attached =
            serde_json::from_value(result).map_err(|_| "Invalid vault attachment response")?;
        let mut aliases = attached.aliases;
        aliases.sort();
        if aliases != scope
            || attached.session.is_empty()
            || attached.session.len() > 4096
            || attached.expires_at <= now
        {
            let _ = transport::exchange(
                &profile.socket,
                json!({
                    "op": "detach", "session": attached.session,
                }),
            )
            .await;
            return Err("Vault grant did not match the requested scope".into());
        }
        let status = VaultStatus {
            aliases,
            expires_at: Some(attached.expires_at),
        };
        entry.parent = None;
        let previous = entry.grant.replace(Grant {
            socket: profile.socket,
            session: attached.session,
            status: status.clone(),
        });
        if let Some(previous) = previous {
            // Install the validated replacement before retiring the old grant.
            // Revocation or expiry of the old broker must not prevent refresh.
            let _ = transport::exchange(
                &previous.socket,
                json!({"op":"detach", "session":previous.session}),
            )
            .await;
        }
        Ok(status)
    }

    pub async fn status(&self, thread_id: ThreadId) -> Result<VaultStatus, String> {
        let mut entries = self.entries.lock().await;
        let Some(owner) = authority(&entries, thread_id) else {
            return Ok(VaultStatus::default());
        };
        let entry = entries.get_mut(&owner).ok_or("Vault is detached")?;
        let grant = entry.grant.as_ref().ok_or("Vault is detached")?;
        let result = transport::exchange(
            &grant.socket,
            json!({"op":"status", "session":grant.session}),
        )
        .await;
        let status = result.and_then(|value| {
            serde_json::from_value::<VaultStatus>(value)
                .map_err(|_| "Invalid vault status response".into())
        });
        match status {
            Ok(status) if status == grant.status => Ok(status),
            _ => {
                entry.grant = None;
                Ok(VaultStatus::default())
            }
        }
    }

    pub async fn detach(&self, thread_id: ThreadId) -> Result<VaultStatus, String> {
        let mut entries = self.entries.lock().await;
        let entry = entries
            .get_mut(&thread_id)
            .ok_or("Vault thread is not active")?;
        entry.parent = None;
        if let Some(pending) = entry.pending.take() {
            let _ = transport::exchange(
                &pending.socket,
                json!({"op":"cancel", "claim_token":pending.approval.claim_token}),
            )
            .await;
        }
        if let Some(grant) = entry.grant.take() {
            // Local authority is removed even when the broker cannot be reached.
            transport::exchange(
                &grant.socket,
                json!({"op":"detach", "session":grant.session}),
            )
            .await?;
        }
        Ok(VaultStatus::default())
    }

    async fn register(&self, thread_id: ThreadId, parent: Option<ThreadId>) {
        let mut entries = self.entries.lock().await;
        if entries.len() < 1024 && !entries.contains_key(&thread_id) {
            entries.insert(
                thread_id,
                Entry {
                    parent,
                    grant: None,
                    pending: None,
                },
            );
        }
    }

    async fn stop(&self, thread_id: ThreadId) {
        let _ = self.detach(thread_id).await;
        self.entries.lock().await.remove(&thread_id);
    }

    async fn request(&self, thread_id: ThreadId, request: Request) -> Result<Value, String> {
        if !matches!(
            request.method.as_str(),
            "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
        ) || !request.path.starts_with('/')
            || request.path.starts_with("//")
            || request.path.len() > 4096
            || request.path.contains(['\r', '\n', '\\'])
            || request.body.as_ref().is_some_and(|body| body.len() > 16384)
        {
            return Err(
                "Invalid vault request; use an allowed method and relative API path".into(),
            );
        }
        let entries = self.entries.lock().await;
        let owner = authority(&entries, thread_id).ok_or("Vault is detached")?;
        let grant = entries
            .get(&owner)
            .and_then(|entry| entry.grant.as_ref())
            .ok_or("Vault is detached")?;
        if !grant.status.aliases.contains(&request.credential) {
            return Err("Credential is outside this session's vault scope".into());
        }
        let value = transport::exchange(
            &grant.socket,
            json!({
                "op":"request", "session":grant.session,
                "credential":request.credential, "method":request.method,
                "path":request.path, "body":request.body,
            }),
        )
        .await?;
        // Never forward arbitrary broker fields into model-visible output.
        #[derive(Deserialize, Serialize)]
        struct Response {
            status: u16,
            body: String,
            truncated: bool,
        }
        let response: Response =
            serde_json::from_value(value).map_err(|_| "Invalid vault response")?;
        if response.body.len() > 8192
            || serde_json::to_vec(&response)
                .map_err(|_| "Invalid vault response")?
                .len()
                > 9000
        {
            return Err("Vault response is too large; narrow the API request".into());
        }
        serde_json::to_value(response).map_err(|_| "Invalid vault response".into())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    credential: String,
    method: String,
    path: String,
    body: Option<String>,
}

fn authority(entries: &HashMap<ThreadId, Entry>, mut id: ThreadId) -> Option<ThreadId> {
    for _ in 0..64 {
        let entry = entries.get(&id)?;
        if entry.grant.is_some() {
            return Some(id);
        }
        id = entry.parent?;
    }
    None
}

fn unix_time() -> Result<i64, String> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "Cannot determine vault expiry")?
        .as_secs();
    i64::try_from(seconds).map_err(|_| "Cannot determine vault expiry".into())
}

fn validate_aliases(aliases: &[String]) -> Result<(), String> {
    if aliases.is_empty()
        || aliases.len() > 8
        || aliases.iter().any(|alias| {
            alias.is_empty()
                || alias.len() > 32
                || !alias
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
    {
        return Err(
            "Vault scope requires 1–8 aliases of at most 32 ASCII letters, digits, _, or -".into(),
        );
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "vault_tests.rs"]
mod tests;
