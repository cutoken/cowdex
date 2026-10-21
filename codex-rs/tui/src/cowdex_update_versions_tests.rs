use super::*;
use pretty_assertions::assert_eq;

#[test]
fn cowdex_revisions_compare_numerically_after_the_upstream_version() {
    for (latest, current, expected) in [
        ("0.163.0-r1", "0.163.0", true),
        ("0.163.0-r10", "0.163.0-r9", true),
        ("0.163.0-r2", "0.163.0-r2", false),
        ("0.163.0", "0.163.0-r1", false),
        ("0.164.0", "0.163.0-r99", true),
    ] {
        assert_eq!(is_newer(latest, current), Some(expected));
    }
    assert_eq!(
        extract_version_from_latest_tag("cowdex-v0.163.0-r2").unwrap(),
        "0.163.0-r2"
    );
}
