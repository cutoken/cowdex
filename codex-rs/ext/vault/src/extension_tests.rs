use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn resumed_vault_guidance_refreshes_when_wording_changes_but_authority_does_not() {
    let extension = VaultExtension(Arc::new(VaultService::new()));
    let store = ExtensionData::new("vault-resume-test");
    let sections = extension
        .contribute_world_state(WorldStateContributionInput {
            thread_id: ThreadId::new(),
            turn_id: "resume",
            environments: &[],
            ready_selected_capability_roots: &[],
            executor_capability_discovery: None,
            extension_metrics: None,
            session_store: &store,
            thread_store: &store,
            turn_store: &store,
        })
        .await;
    let section = &sections[0];
    let legacy = serde_json::json!({"aliases": [], "expires_at": null});
    let expected = section.render_diff(PreviousWorldStateSection::Unknown);
    assert!(expected.is_some());
    assert_eq!(
        section.render_diff(PreviousWorldStateSection::Known(&legacy)),
        expected
    );
    let mut previous = section.snapshot().clone();
    previous["instructions"] = "Previous vault guidance".into();
    assert_eq!(
        section.render_diff(PreviousWorldStateSection::Known(&previous)),
        expected
    );
    assert_eq!(
        section.render_diff(PreviousWorldStateSection::Known(section.snapshot())),
        None
    );
    assert_eq!(section.render_diff(PreviousWorldStateSection::Absent), None);
}
