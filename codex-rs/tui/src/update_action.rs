/// Update action the CLI should perform after the TUI exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    /// Run Cowdex's release installer with Python on Unix.
    StandaloneUnix,
    /// Run Cowdex's release installer with the Python launcher on Windows.
    StandaloneWindows,
}

impl UpdateAction {
    /// Returns the command for downloading and running Cowdex's latest installer.
    pub fn command_args(self) -> (&'static str, &'static [&'static str]) {
        const INSTALL: &str = "import urllib.request;exec(compile(urllib.request.urlopen('https://github.com/cutoken/cowdex/releases/latest/download/install.py', timeout=60).read(), '<cowdex installer>', 'exec'), {'__name__':'__main__','__file__':'install.py'})";
        match self {
            Self::StandaloneUnix => ("python3", &["-c", INSTALL, "--repo", "cutoken/cowdex"]),
            Self::StandaloneWindows => ("py", &["-3", "-c", INSTALL, "--repo", "cutoken/cowdex"]),
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
