use super::RecursiveRmGuardScope;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct LexicalPath {
    root: String,
    components: Vec<String>,
    windows: bool,
}

pub(super) fn protected_paths(scope: &RecursiveRmGuardScope) -> Vec<LexicalPath> {
    let mut paths = Vec::new();
    if let Some(mut root) = normalize_path(&scope.execution_cwd, &scope.execution_cwd) {
        root.components.clear();
        paths.push(root);
    }
    paths.extend(
        std::iter::once(scope.execution_cwd.as_str())
            .chain(scope.home.as_deref())
            .chain(scope.workspace_roots.iter().map(String::as_str))
            .filter_map(|path| normalize_path(path, &scope.execution_cwd)),
    );
    paths
}

pub(super) fn normalize_path(path: &str, cwd: &str) -> Option<LexicalPath> {
    let windows = is_windows_path(cwd);
    let (root, components, absolute) = split_path(path, windows)?;
    let mut normalized = if absolute {
        LexicalPath {
            root,
            components: Vec::new(),
            windows,
        }
    } else {
        let (cwd_root, cwd_components, cwd_absolute) = split_path(cwd, windows)?;
        if !cwd_absolute {
            return None;
        }
        LexicalPath {
            root: cwd_root,
            components: cwd_components,
            windows,
        }
    };
    for component in components {
        match component.as_str() {
            "" | "." => {}
            ".." => {
                normalized.components.pop();
            }
            _ => normalized.components.push(if windows {
                component.to_ascii_lowercase()
            } else {
                component
            }),
        }
    }
    Some(normalized)
}

pub(super) fn is_relative_path(path: &str, cwd: &str) -> bool {
    if is_windows_path(cwd) {
        let normalized = path.replace('\\', "/");
        normalized.as_bytes().get(1) != Some(&b':') && !normalized.starts_with('/')
    } else {
        !path.starts_with('/')
    }
}

pub(super) fn glob_is_rooted_at_protected_path(
    path: &str,
    cwd: &str,
    protected_paths: &[LexicalPath],
) -> bool {
    let glob_byte = path
        .char_indices()
        .find_map(|(index, character)| matches!(character, '*' | '?' | '[').then_some(index));
    let Some(glob_byte) = glob_byte else {
        return false;
    };
    let prefix = &path[..glob_byte];
    let component_prefix = prefix.rsplit(['/', '\\']).next().unwrap_or(prefix);
    let prefix = if component_prefix == ".." {
        &prefix[..prefix.len() - component_prefix.len()]
    } else {
        prefix.trim_end_matches(['/', '\\'])
    };
    let prefix = if prefix.is_empty() { "." } else { prefix };
    normalize_path(prefix, cwd)
        .is_some_and(|path| protected_paths.iter().any(|protected| protected == &path))
}

fn split_path(path: &str, windows: bool) -> Option<(String, Vec<String>, bool)> {
    if windows {
        let normalized = path.replace('\\', "/");
        let bytes = normalized.as_bytes();
        if bytes.get(1) == Some(&b':') {
            if bytes.get(2) != Some(&b'/') {
                return None;
            }
            let root = normalized[..2].to_ascii_lowercase();
            return Some((root, split_components(&normalized[2..]), true));
        }
        if let Some(stripped) = normalized.strip_prefix("//") {
            let mut components = split_components(stripped);
            if components.len() < 2 {
                return None;
            }
            let share = components.drain(..2).collect::<Vec<_>>().join("/");
            return Some((
                format!("//{}", share.to_ascii_lowercase()),
                components,
                true,
            ));
        }
        if normalized.starts_with('/') {
            return None;
        }
        Some((String::new(), split_components(&normalized), false))
    } else {
        Some((
            "/".to_string(),
            split_components(path),
            path.starts_with('/'),
        ))
    }
}

fn split_components(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_windows_path(path: &str) -> bool {
    path.as_bytes().get(1) == Some(&b':') || path.starts_with("\\\\")
}
