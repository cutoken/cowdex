use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn reserve_enters_only_for_an_authorized_account_and_recovers_its_previous_model() {
    let normal: ModelPreset = codex_models_manager::bundled_models_response()
        .unwrap()
        .models
        .into_iter()
        .find(|model| model.slug == "gpt-6-sol")
        .unwrap()
        .into();
    let mut reserve = normal.clone();
    reserve.model = RESERVE_MODEL.to_string();
    reserve.show_in_picker = false;
    let models = vec![normal.clone(), reserve.clone()];
    insta::assert_snapshot!(
        "reserve_transition_notices",
        format!("{}\n{}", reserve_notice(&reserve), reserve_notice(&normal))
    );

    let mut usage: GetAccountRateLimitsResponse = serde_json::from_value(json!({
        "accountId": "account-a", "userId": "user-a", "ordinaryUsageAllowed": false,
        "rateLimitUpsell": {"banner_type": "luna_reserve", "blocked_model_slug": "gpt-6-sol"},
        "rateLimits": {}, "rateLimitsByLimitId": null, "rateLimitResetCredits": null
    }))
    .unwrap();
    let previous = ReserveReturnModel {
        account_id: "account-a".into(),
        model: normal.model.clone(),
        effort: None,
    };
    assert_eq!(
        reserve_target(&usage, &normal.model, None, &models),
        Some(&reserve)
    );
    usage.account_id = None;
    assert_eq!(reserve_target(&usage, &normal.model, None, &models), None);
    usage.account_id = Some("account-a".into());
    usage.rate_limit_upsell = None;
    // The existing ordinary usage state remains exhausted until the backend confirms recovery.
    assert_eq!(
        reserve_target(&usage, RESERVE_MODEL, Some(&previous), &models),
        None
    );
    usage.ordinary_usage_allowed = Some(true);
    assert_eq!(
        reserve_target(&usage, RESERVE_MODEL, Some(&previous), &models),
        Some(&normal)
    );
    usage.account_id = Some("another-account".into());
    assert_eq!(
        reserve_target(&usage, RESERVE_MODEL, Some(&previous), &models),
        None
    );
}

#[test]
fn reserve_return_survives_resume() {
    let home = tempfile::tempdir().unwrap();
    let thread_id = ThreadId::new();
    let previous = ReserveReturnModel {
        account_id: "account-a".into(),
        model: "gpt-6-sol".into(),
        effort: None,
    };
    previous.save(home.path(), thread_id).unwrap();
    let restored = ReserveReturnModel::load(home.path(), thread_id).unwrap();
    assert_eq!(
        serde_json::to_value(restored).unwrap(),
        serde_json::to_value(previous).unwrap()
    );
    ReserveReturnModel::clear(home.path(), thread_id);
    assert!(ReserveReturnModel::load(home.path(), thread_id).is_none());
}
