/// Update action the CLI should perform after the TUI exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Run Cowdex's release installer with the Unix shell.
    StandaloneUnix,
    /// Run Cowdex's release installer with PowerShell on Windows.
    StandaloneWindows,
}

impl UpdateAction {
    /// Returns the command for downloading and running Cowdex's latest installer.
    pub fn command_args(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::StandaloneUnix => (
                "sh",
                &[
                    "-c",
                    "curl -fsSL https://raw.githubusercontent.com/cutoken/cowdex/main/scripts/install.sh | sh",
                ],
            ),
            Self::StandaloneWindows => (
                "powershell",
                &[
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    "irm https://raw.githubusercontent.com/cutoken/cowdex/main/scripts/install.ps1 | iex",
                ],
            ),
        }
    }

    /// Returns the public command shown in update prompts.
    pub fn command_str(self) -> String {
        "cowdex update".to_string()
    }
}

pub fn get_update_action() -> Option<UpdateAction> {
    Some(if cfg!(windows) {
        UpdateAction::StandaloneWindows
    } else {
        UpdateAction::StandaloneUnix
    })
}
