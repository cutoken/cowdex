use super::*;
use codex_protocol::protocol::SubAgentSource;
use pretty_assertions::assert_eq;

async fn test_turn_context(
    slug: &str,
    source: SessionSource,
    context_window: i64,
    max_context_window: i64,
    use_max_context_window: bool,
) -> TurnContext {
    let (_session, mut turn_context) = crate::session::tests::make_session_and_context().await;
    turn_context.model_info.slug = slug.to_string();
    turn_context.model_info.context_window = Some(context_window);
    turn_context.model_info.max_context_window = Some(max_context_window);
    turn_context.model_info.effective_context_window_percent = 100;
    turn_context.model_info.auto_compact_token_limit = Some(max_context_window);
    turn_context.session_source = source;
    Arc::make_mut(&mut turn_context.config).use_max_context_window = use_max_context_window;
    turn_context
}

#[tokio::test]
async fn astra_root_caps_max_context_and_compaction_at_400k() {
    let turn_context = test_turn_context(
        "gpt-6-astra",
        SessionSource::Exec,
        /*context_window*/ 272_000,
        /*max_context_window*/ 872_000,
        /*use_max_context_window*/ true,
    )
    .await;

    assert_eq!(turn_context.model_context_window(), Some(400_000));
    assert_eq!(turn_context.auto_compact_token_limit(), Some(400_000));
}

#[tokio::test]
async fn non_root_agent_ignores_max_context_and_caps_inherited_override() {
    let source = SessionSource::SubAgent(SubAgentSource::Other("test".to_string()));
    let turn_context = test_turn_context(
        "gpt-6-astra",
        source,
        /*context_window*/ 700_000,
        /*max_context_window*/ 872_000,
        /*use_max_context_window*/ true,
    )
    .await;

    assert_eq!(turn_context.model_context_window(), Some(258_400));
    assert_eq!(turn_context.auto_compact_token_limit(), Some(258_400));
}

#[tokio::test]
async fn non_root_agent_preserves_default_compaction_threshold() {
    let source = SessionSource::SubAgent(SubAgentSource::Other("test".to_string()));
    let mut turn_context = test_turn_context(
        "gpt-6-astra",
        source,
        /*context_window*/ 272_000,
        /*max_context_window*/ 872_000,
        /*use_max_context_window*/ false,
    )
    .await;
    turn_context.model_info.effective_context_window_percent = 95;
    turn_context.model_info.auto_compact_token_limit = None;

    assert_eq!(turn_context.model_context_window(), Some(258_400));
    assert_eq!(turn_context.auto_compact_token_limit(), Some(244_800));
}

#[tokio::test]
async fn smaller_windows_and_non_astra_roots_remain_unchanged() {
    let astra = test_turn_context(
        "gpt-6-astra",
        SessionSource::Exec,
        /*context_window*/ 320_000,
        /*max_context_window*/ 872_000,
        /*use_max_context_window*/ false,
    )
    .await;
    let other = test_turn_context(
        "gpt-5.4",
        SessionSource::Exec,
        /*context_window*/ 600_000,
        /*max_context_window*/ 872_000,
        /*use_max_context_window*/ false,
    )
    .await;

    assert_eq!(astra.model_context_window(), Some(320_000));
    assert_eq!(other.model_context_window(), Some(600_000));
}
