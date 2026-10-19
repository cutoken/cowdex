use super::*;

fn scope() -> RecursiveRmGuardScope {
    RecursiveRmGuardScope {
        execution_cwd: "/work/project".to_string(),
        home: Some("/home/user".to_string()),
        workspace_roots: vec!["/work/project".to_string(), "/work/other".to_string()],
    }
}

fn command(words: &[&str]) -> Vec<String> {
    words.iter().map(ToString::to_string).collect()
}

#[test]
fn rejects_unsafe_recursive_rm_invocations() {
    let commands = [
        command(&["rm", "-r", "--", "/"]),
        command(&["rm", "-R", "/home/user"]),
        command(&["rm", "--recursive", "/work/other"]),
        command(&["rm", "-fr", "src/.."]),
        command(&["bash", "-lc", "rm -r child/../*"]),
        command(&["bash", "-lc", "rm -r ..?*"]),
        command(&["sh", "-c", "rm -r \"${TARGET:?}\""]),
        command(&["sh", "-c", "rm -r prefix/$TARGET"]),
        command(&["sh", "-c", "rm -r $(find-target)"]),
        command(&["bash", "-lc", "cd /tmp; rm -r build"]),
        command(&["bash", "-lc", "pushd /tmp; rm -r build"]),
        command(&["bash", "-lc", "popd; rm -r build"]),
        command(&["bash", "-lc", "cd /home; (rm -r user)"]),
        command(&["sudo", "-n", "rm", "-r", "/"]),
        command(&["sudo", "-u", "root", "rm", "-r", "/"]),
        command(&["env", "MODE=test", "rm", "-r", "/"]),
        command(&["command", "--", "rm", "-r", "/"]),
        command(&["bash", "-lc", "sh -c 'rm -r /'"]),
    ];
    for command in commands {
        assert!(
            recursive_rm_guard_violation(&command, &scope()).is_some(),
            "unsafe command unexpectedly allowed: {command:?}"
        );
    }
}

#[test]
fn allows_literal_descendants_and_non_recursive_rm() {
    let commands = [
        command(&["rm", "-r", "target"]),
        command(&["rm", "-r", "/work/project/target"]),
        command(&["bash", "-lc", "rm -r target/*"]),
        command(&["bash", "-lc", "rm -r '*'"]),
        command(&["bash", "-lc", "rm -r build; cd /tmp"]),
        command(&["bash", "-lc", "echo $(cd /tmp); rm -r target"]),
        command(&["rm", "-f", "/"]),
    ];
    for command in commands {
        assert!(recursive_rm_guard_violation(&command, &scope()).is_none());
    }
}
