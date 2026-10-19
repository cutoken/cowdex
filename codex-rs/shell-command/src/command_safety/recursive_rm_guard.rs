use std::collections::HashSet;
use std::fmt;

use tree_sitter::Node;

use crate::bash::extract_bash_command;
use crate::bash::try_parse_shell;
use crate::command_safety::is_dangerous_command::executable_name_lookup_key;

mod lexical_path;

use lexical_path::glob_is_rooted_at_protected_path;
use lexical_path::is_relative_path;
use lexical_path::normalize_path;
use lexical_path::protected_paths;

/// Paths that a recursive `rm` command must not target directly.
pub struct RecursiveRmGuardScope {
    pub execution_cwd: String,
    pub home: Option<String>,
    pub workspace_roots: Vec<String>,
}

/// Why a recursive `rm` invocation cannot be executed.
pub struct RecursiveRmGuardViolation;

impl fmt::Display for RecursiveRmGuardViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("recursive rm command cannot be executed safely")
    }
}

/// Returns a violation when a statically visible recursive `rm` could remove a
/// protected root. This function never evaluates shell expressions, expands
/// environment variables or globs, or resolves filesystem links.
pub fn recursive_rm_guard_violation(
    command: &[String],
    scope: &RecursiveRmGuardScope,
) -> Option<RecursiveRmGuardViolation> {
    if let Some((_, script)) = extract_bash_command(command) {
        return check_script(script, scope, /*cwd_changed*/ false);
    }

    let words = command
        .iter()
        .map(|word| ParsedWord {
            literal: (!looks_dynamic(word)).then(|| word.clone()),
            has_unquoted_glob: has_unescaped_glob(word),
        })
        .collect::<Vec<_>>();
    check_words(&words, scope, /*cwd_changed*/ false)
}

struct ParsedWord {
    literal: Option<String>,
    has_unquoted_glob: bool,
}

fn check_script(
    script: &str,
    scope: &RecursiveRmGuardScope,
    cwd_changed: bool,
) -> Option<RecursiveRmGuardViolation> {
    let tree = try_parse_shell(script)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }

    let mut changed_scopes = HashSet::new();
    if cwd_changed {
        changed_scopes.insert(ExecutionScope::TopLevel);
    }
    for node in command_nodes(root) {
        let Some(words) = parsed_command_words(node, script) else {
            continue;
        };
        let execution_scopes = execution_scopes(node);
        let cwd_changed = execution_scopes
            .iter()
            .any(|scope| changed_scopes.contains(scope));
        if let Some(violation) = check_words(&words, scope, cwd_changed) {
            return Some(violation);
        }
        if command_changes_cwd(&words) {
            changed_scopes.insert(
                execution_scopes
                    .get(1)
                    .copied()
                    .unwrap_or(ExecutionScope::TopLevel),
            );
        }
    }
    None
}

fn command_changes_cwd(words: &[ParsedWord]) -> bool {
    words
        .first()
        .and_then(|word| word.literal.as_deref())
        .and_then(executable_name_lookup_key)
        .is_some_and(|name| matches!(name.as_str(), "cd" | "pushd" | "popd"))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ExecutionScope {
    TopLevel,
    Nested(usize, usize),
}

fn execution_scopes(mut node: Node<'_>) -> Vec<ExecutionScope> {
    let mut scopes = vec![ExecutionScope::TopLevel];
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "command_substitution" | "subshell") {
            scopes.push(ExecutionScope::Nested(
                parent.start_byte(),
                parent.end_byte(),
            ));
        }
        node = parent;
    }
    scopes
}

fn command_nodes(root: Node<'_>) -> Vec<Node<'_>> {
    let mut nodes = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "command" {
            nodes.push(node);
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    nodes.sort_by_key(Node::start_byte);
    nodes
}

fn parsed_command_words(command: Node<'_>, source: &str) -> Option<Vec<ParsedWord>> {
    let mut words = Vec::new();
    let mut found_command_name = false;
    let mut cursor = command.walk();
    for child in command.named_children(&mut cursor) {
        if child.kind() == "command_name" {
            words.push(parsed_shell_word(child.named_child(0)?, source));
            found_command_name = true;
        } else if found_command_name && !is_non_argument_kind(child.kind()) {
            words.push(parsed_shell_word(child, source));
        }
    }
    found_command_name.then_some(words)
}

fn is_non_argument_kind(kind: &str) -> bool {
    matches!(
        kind,
        "comment"
            | "file_redirect"
            | "heredoc_redirect"
            | "herestring_redirect"
            | "heredoc_body"
            | "simple_heredoc_body"
    )
}

fn parsed_shell_word(node: Node<'_>, source: &str) -> ParsedWord {
    let raw = node.utf8_text(source.as_bytes()).unwrap_or_default();
    let mut parsed = ParsedWord {
        literal: None,
        has_unquoted_glob: false,
    };
    match node.kind() {
        "word" | "number" if node.named_child_count() == 0 => {
            parsed.literal = (!looks_dynamic(raw)).then(|| unescape_word(raw));
            parsed.has_unquoted_glob = has_unescaped_glob(raw);
        }
        "raw_string" => {
            parsed.literal = raw
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
                .map(str::to_string);
        }
        "string" if !has_dynamic_descendant(node) => {
            parsed.literal = raw
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .map(unescape_double_quoted);
        }
        "concatenation" if !has_dynamic_descendant(node) => {
            let mut literal = String::new();
            let mut cursor = node.walk();
            for part in node.named_children(&mut cursor) {
                let part = parsed_shell_word(part, source);
                let Some(value) = part.literal else {
                    return parsed;
                };
                literal.push_str(&value);
                parsed.has_unquoted_glob |= part.has_unquoted_glob;
            }
            parsed.literal = Some(literal);
        }
        _ => {}
    }
    parsed
}

fn has_dynamic_descendant(node: Node<'_>) -> bool {
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        if matches!(
            current.kind(),
            "simple_expansion"
                | "expansion"
                | "command_substitution"
                | "arithmetic_expansion"
                | "process_substitution"
                | "brace_expression"
                | "brace_expansion"
                | "extglob_pattern"
        ) {
            return true;
        }
        let mut cursor = current.walk();
        for child in current.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

fn check_words(
    words: &[ParsedWord],
    scope: &RecursiveRmGuardScope,
    cwd_changed: bool,
) -> Option<RecursiveRmGuardViolation> {
    let command_name = words.first()?.literal.as_deref()?;
    match executable_name_lookup_key(command_name).as_deref() {
        Some("rm") => check_rm(&words[1..], scope, cwd_changed),
        Some("sudo") => check_words(
            words.get(sudo_command_index(&words[1..]) + 1..)?,
            scope,
            cwd_changed,
        ),
        Some("command") => {
            let command_index = words[1..].iter().position(|word| {
                word.literal
                    .as_deref()
                    .is_some_and(|argument| !argument.starts_with('-'))
            })?;
            let nested = words.get(command_index + 1..)?;
            check_words(nested, scope, cwd_changed)
        }
        Some("env") => {
            let command_index = env_command_index(&words[1..]);
            check_words(words.get(command_index + 1..)?, scope, cwd_changed)
        }
        Some("sh") | Some("bash") | Some("zsh") => {
            check_script(shell_script_operand(&words[1..])?, scope, cwd_changed)
        }
        _ => None,
    }
}

fn sudo_command_index(args: &[ParsedWord]) -> usize {
    const OPTIONS_WITH_VALUE: &[&str] = &[
        "-C", "-D", "-g", "-h", "-p", "-R", "-r", "-T", "-t", "-U", "-u", "--chdir", "--group",
        "--host", "--prompt", "--role", "--type", "--user",
    ];
    let mut index = 0;
    while let Some(argument) = args.get(index).and_then(|word| word.literal.as_deref()) {
        if argument == "--" {
            return index + 1;
        }
        if !argument.starts_with('-') || argument == "-" {
            break;
        }
        index += if OPTIONS_WITH_VALUE.contains(&argument) {
            2
        } else {
            1
        };
    }
    index
}

fn env_command_index(args: &[ParsedWord]) -> usize {
    let mut index = 0;
    while let Some(argument) = args.get(index).and_then(|word| word.literal.as_deref()) {
        if argument == "--" {
            return index + 1;
        }
        if matches!(argument, "-i" | "--ignore-environment")
            || argument
                .split_once('=')
                .is_some_and(|(name, _)| !name.is_empty() && !name.starts_with('-'))
        {
            index += 1;
        } else {
            break;
        }
    }
    index
}

fn shell_script_operand(args: &[ParsedWord]) -> Option<&str> {
    let flag_index = args
        .iter()
        .position(|word| matches!(word.literal.as_deref(), Some("-c" | "-lc")))?;
    args.get(flag_index + 1)?.literal.as_deref()
}

fn check_rm(
    args: &[ParsedWord],
    scope: &RecursiveRmGuardScope,
    cwd_changed: bool,
) -> Option<RecursiveRmGuardViolation> {
    let recursive = args
        .iter()
        .take_while(|word| word.literal.as_deref() != Some("--"))
        .filter_map(|word| word.literal.as_deref())
        .any(is_recursive_option);
    if !recursive {
        return None;
    }

    let mut options_ended = false;
    for operand in args {
        if operand.literal.as_deref() == Some("--") && !options_ended {
            options_ended = true;
            continue;
        }
        if !options_ended
            && operand
                .literal
                .as_deref()
                .is_some_and(|value| value.starts_with('-') && value != "-")
        {
            continue;
        }
        let Some(literal) = operand.literal.as_deref() else {
            return Some(RecursiveRmGuardViolation);
        };
        if cwd_changed && is_relative_path(literal, &scope.execution_cwd) {
            return Some(RecursiveRmGuardViolation);
        }

        let protected_paths = protected_paths(scope);
        let normalized = normalize_path(literal, &scope.execution_cwd);
        if normalized.is_none() {
            return Some(RecursiveRmGuardViolation);
        }
        if normalized
            .as_ref()
            .is_some_and(|path| protected_paths.iter().any(|protected| protected == path))
        {
            return Some(RecursiveRmGuardViolation);
        }
        if operand.has_unquoted_glob
            && glob_is_rooted_at_protected_path(literal, &scope.execution_cwd, &protected_paths)
        {
            return Some(RecursiveRmGuardViolation);
        }
    }
    None
}

fn is_recursive_option(argument: &str) -> bool {
    argument == "--recursive"
        || argument.strip_prefix('-').is_some_and(|flags| {
            !flags.starts_with('-') && flags.chars().any(|flag| matches!(flag, 'r' | 'R'))
        })
}

fn looks_dynamic(word: &str) -> bool {
    word.starts_with('~')
        || word.contains('$')
        || word.contains('`')
        || word.contains("<(")
        || word.contains(">(")
        || word.contains('{')
        || word.contains('}')
}

fn has_unescaped_glob(word: &str) -> bool {
    let mut escaped = false;
    for character in word.chars() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if matches!(character, '*' | '?' | '[') {
            return true;
        }
    }
    false
}

fn unescape_word(word: &str) -> String {
    let mut unescaped = String::new();
    let mut characters = word.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            if let Some(next) = characters.next() {
                unescaped.push(next);
            }
        } else {
            unescaped.push(character);
        }
    }
    unescaped
}

fn unescape_double_quoted(word: &str) -> String {
    let mut unescaped = String::new();
    let mut characters = word.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next() {
                Some(next @ ('"' | '$' | '`' | '\\')) => unescaped.push(next),
                Some(next) => {
                    unescaped.push('\\');
                    unescaped.push(next);
                }
                None => unescaped.push('\\'),
            }
        } else {
            unescaped.push(character);
        }
    }
    unescaped
}

#[cfg(test)]
#[path = "recursive_rm_guard_tests.rs"]
mod tests;
