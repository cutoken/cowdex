use super::*;
use pretty_assertions::assert_eq;

#[test]
fn attaches_using_browser_defaults_or_narrower_scope() {
    assert_eq!(
        VaultCommand::parse("attach"),
        Ok(VaultCommand::Attach {
            profile: String::new(),
            scope: Vec::new(),
        })
    );
    assert_eq!(
        VaultCommand::parse("attach --scope cf"),
        Ok(VaultCommand::Attach {
            profile: String::new(),
            scope: vec!["cf".into()],
        })
    );
}

#[test]
fn parses_scoped_profile_with_spaces() {
    assert_eq!(
        VaultCommand::parse("attach '/tmp/my vault.json' --scope cf,github"),
        Ok(VaultCommand::Attach {
            profile: "/tmp/my vault.json".to_string(),
            scope: vec!["cf".to_string(), "github".to_string()],
        })
    );
}

#[test]
fn rejects_missing_or_unbounded_scope() {
    for input in [
        "attach profile.json",
        "attach profile.json --scope cf,",
        "attach profile.json --scope a,b,c,d,e,f,g,h,i",
        "attach profile.json --scope $(secret)",
        "attach profile.json --scope cf,cf",
        "attach profile.json --scope cf:dev",
    ] {
        assert_eq!(VaultCommand::parse(input), Err(VAULT_USAGE));
    }
}

#[test]
fn scoped_status_snapshot() {
    insta::assert_snapshot!(status_text(&ThreadVaultStatusResponse {
        aliases: vec!["cf".to_string(), "github".to_string()],
        expires_at: Some(1_800_000_000),
    }), @"
    Vault attached: cf, github
    Expires at: 2027-01-15 08:00:00 UTC.
    Available to this session and its subagents through the vault request tool.
    ");
}

#[test]
fn detached_status_snapshot() {
    insta::assert_snapshot!(status_text(&ThreadVaultStatusResponse { aliases: vec![], expires_at: None }), @"Vault detached. No credentials are available through the vault.");
}

#[test]
fn pending_approval_snapshot_and_error_boundary() {
    let message = "Vault approval required: ABCDEF123456. Approve the matching request in the vault page, then repeat the same /vault attach command, including its options.";
    let error = |method: &str, code, message: &str| TypedRequestError::Server {
        method: method.into(),
        source: codex_app_server_protocol::JSONRPCErrorError {
            code,
            message: message.into(),
            data: None,
        },
    };
    insta::assert_snapshot!(approval_text(&error("thread/vault/attach", /*code*/ -32600, message)).unwrap(), @"
    Vault approval required: ABCDEF123456
    Compare this code in the vault page and approve the requested credentials.
    Then repeat the same /vault attach command, including its options. Access lasts up to one hour.
    ");
    for invalid in [
        error("thread/vault/status", /*code*/ -32600, message),
        error("thread/vault/attach", /*code*/ -32603, message),
        error(
            "thread/vault/attach",
            /*code*/ -32600,
            &message.replace("ABCDEF123456", "private-token"),
        ),
        error(
            "thread/vault/attach",
            /*code*/ -32600,
            &format!("{message} private-token"),
        ),
    ] {
        assert_eq!(approval_text(&invalid), None);
    }
}
