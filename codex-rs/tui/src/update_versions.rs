use codex_install_context::InstallContext;

/// Compare Cowdex releases, including numbered revisions of the same upstream version.
pub fn is_newer(latest: &str, current: &str) -> Option<bool> {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => Some(l > c),
        _ => None,
    }
}

pub(crate) fn extract_version_from_latest_tag(latest_tag_name: &str) -> anyhow::Result<String> {
    latest_tag_name
        .strip_prefix("cowdex-v")
        .filter(|version| parse_version(version).is_some())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse latest tag name '{latest_tag_name}'"))
}

pub(crate) fn is_source_build_version(version: &str) -> bool {
    parse_version(version)
        .is_some_and(|(major, minor, patch, _)| (major, minor, patch) == (0, 0, 0))
}

/// Read Cowdex's installed release revision without changing its upstream backend version.
pub fn current_release_version() -> String {
    let upstream = crate::version::CODEX_CLI_VERSION;
    InstallContext::current()
        .package_manifest()
        .and_then(|manifest| manifest.release_version)
        .filter(|release| {
            matches!((parse_version(release), parse_version(upstream)),
                (Some((major, minor, patch, _)), Some((up_major, up_minor, up_patch, _)))
                    if (major, minor, patch) == (up_major, up_minor, up_patch))
        })
        .unwrap_or_else(|| upstream.to_string())
}

fn parse_version(value: &str) -> Option<(u64, u64, u64, u64)> {
    let (version, revision) = match value.trim().split_once("-r") {
        Some((version, revision)) => (version, revision.parse().ok()?),
        None => (value.trim(), 0),
    };
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, revision))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn extracts_version_from_latest_tag() {
        assert_eq!(
            extract_version_from_latest_tag("cowdex-v1.5.0").expect("failed to parse version"),
            "1.5.0"
        );
    }

    #[test]
    fn latest_tag_without_prefix_is_invalid() {
        assert!(extract_version_from_latest_tag("v1.5.0").is_err());
    }

    #[test]
    fn prerelease_version_is_not_considered_newer() {
        assert_eq!(is_newer("0.11.0-beta.1", "0.11.0"), None);
        assert_eq!(is_newer("1.0.0-rc.1", "1.0.0"), None);
    }

    #[test]
    fn plain_semver_comparisons_work() {
        assert_eq!(is_newer("0.11.1", "0.11.0"), Some(true));
        assert_eq!(is_newer("0.11.0", "0.11.1"), Some(false));
        assert_eq!(is_newer("1.0.0", "0.9.9"), Some(true));
        assert_eq!(is_newer("0.9.9", "1.0.0"), Some(false));
    }

    #[test]
    fn source_build_version_is_not_checked() {
        assert!(is_source_build_version("0.0.0"));
        assert!(!is_source_build_version("0.1.0"));
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(parse_version(" 1.2.3 \n"), Some((1, 2, 3, 0)));
        assert_eq!(is_newer(" 1.2.3 ", "1.2.2"), Some(true));
    }
}

#[cfg(test)]
#[path = "cowdex_update_versions_tests.rs"]
mod cowdex_tests;
