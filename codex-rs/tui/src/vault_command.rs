use codex_app_server_client::TypedRequestError;
use codex_app_server_protocol::ThreadVaultStatusResponse;

pub(crate) const VAULT_USAGE: &str = "Usage: /vault attach [--scope alias[,alias...]]\n       /vault attach <profile-path> --scope alias[,alias...]\n       /vault status\n       /vault detach\nUnlock the vault page first. With no arguments, attach uses the credentials selected there.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VaultCommand {
    Attach { profile: String, scope: Vec<String> },
    Status,
    Detach,
}

impl VaultCommand {
    pub(crate) fn parse(args: &str) -> Result<Self, &'static str> {
        let words = shlex::split(args).ok_or(VAULT_USAGE)?;
        let (profile, scope) = match words.as_slice() {
            [command] if command == "status" => return Ok(Self::Status),
            [command] if command == "detach" => return Ok(Self::Detach),
            [command] if command == "attach" => {
                return Ok(Self::Attach {
                    profile: String::new(),
                    scope: Vec::new(),
                });
            }
            [command, flag, scope] if command == "attach" && flag == "--scope" => {
                (String::new(), scope)
            }
            [command, profile, flag, scope]
                if command == "attach" && flag == "--scope" && !profile.is_empty() =>
            {
                (profile.clone(), scope)
            }
            _ => return Err(VAULT_USAGE),
        };
        let scope = scope.split(',').map(str::to_owned).collect::<Vec<_>>();
        if scope.is_empty()
            || scope.len() > 8
            || scope.iter().any(|alias| {
                alias.is_empty()
                    || alias.len() > 32
                    || !alias
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            })
            || scope
                .iter()
                .enumerate()
                .any(|(index, alias)| scope[..index].contains(alias))
        {
            return Err(VAULT_USAGE);
        }
        Ok(Self::Attach { profile, scope })
    }
}

pub(crate) fn status_text(status: &ThreadVaultStatusResponse) -> String {
    if status.aliases.is_empty() {
        return "Vault detached. No credentials are available through the vault.".to_string();
    }
    let aliases = status.aliases.join(", ");
    let expiry = status
        .expires_at
        .and_then(|expires_at| chrono::DateTime::from_timestamp(expires_at, /*nsecs*/ 0))
        .map_or_else(
            || "unknown".to_string(),
            |expires_at| expires_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        );
    format!(
        "Vault attached: {aliases}\nExpires at: {expiry}.\nAvailable to this session and its subagents through the vault request tool."
    )
}

pub(crate) fn approval_text(error: &TypedRequestError) -> Option<String> {
    let TypedRequestError::Server { method, source } = error else {
        return None;
    };
    if method != "thread/vault/attach" || source.code != -32600 {
        return None;
    }
    let code = source.message.strip_prefix("Vault approval required: ")?.strip_suffix(
        ". Approve the matching request in the vault page, then repeat the same /vault attach command, including its options.",
    )?;
    if code.len() != 12
        || !code
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    {
        return None;
    }
    Some(format!(
        "Vault approval required: {code}\nCompare this code in the vault page and approve the requested credentials.\nThen repeat the same /vault attach command, including its options. Access lasts up to 24 hours."
    ))
}

#[cfg(test)]
#[path = "vault_command_tests.rs"]
mod tests;
