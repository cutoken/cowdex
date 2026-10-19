use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn refusal_copy_uses_verified_access() {
    for (programs, expected) in [
        (json!([]), Notice::Apply),
        (
            json!([{"program":"cyber","state":"inactive","grants":[]}]),
            Notice::Apply,
        ),
        (
            json!([{"program":"cyber","state":"unavailable","grants":[]}]),
            Notice::Limited,
        ),
        (
            json!([{"program":"cyber","state":"active","grants":[{"level":"tac2"}]}]),
            Notice::Limited,
        ),
        (
            json!([{"program":"cyber","state":"active","grants":[]}]),
            Notice::Limited,
        ),
        (
            json!([{"program":"cyber","state":"inactive","grants":[{"level":"tac2"}]}]),
            Notice::Limited,
        ),
        (
            json!([{"program":"cyber","state":"unknown","grants":[]}]),
            Notice::Limited,
        ),
    ] {
        let access: VerifiedAccess = serde_json::from_value(json!({"programs": programs})).unwrap();
        assert_eq!(access.notice(), expected);
    }
}

#[test]
fn astra_refusal_copy_takes_precedence_over_eligibility() {
    for model in ["gpt-6-astra", "gpt-6-astra-wm"] {
        assert_eq!(Notice::Apply.for_model(model), Notice::Astra);
        assert_eq!(Notice::Limited.for_model(model), Notice::Astra);
    }
    assert_eq!(Notice::Apply.for_model("gpt-5.6-sol"), Notice::Apply);
    assert_eq!(Notice::Apply.for_model("unmapped-model"), Notice::Limited);
}
