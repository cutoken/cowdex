#[test]
fn cleanup_result_reports_backups_and_skipped_sessions() {
    insta::assert_snapshot!(super::report(12, 3, 104857600, 400, 2));
}
