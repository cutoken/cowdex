mod powershell_parser;
mod recursive_rm_guard;

pub mod is_dangerous_command;
pub mod is_safe_command;
#[cfg(windows)]
pub(crate) mod windows_safe_commands;
pub(crate) use powershell_parser::try_parse_powershell_ast_commands;
pub use recursive_rm_guard::RecursiveRmGuardScope;
pub use recursive_rm_guard::RecursiveRmGuardViolation;
pub use recursive_rm_guard::recursive_rm_guard_violation;
