use super::ContextualUserFragment;
use super::world_state::PreviousSectionState;
use super::world_state::WorldStateHash;
use super::world_state::WorldStateSection;
use std::path::Path;
use std::path::PathBuf;

/// Bundled UI policy shared by the main agent and agents doing UI work.
#[derive(Clone)]
pub(crate) struct UxPolicy {
    path: PathBuf,
}

impl UxPolicy {
    pub(crate) fn new(codex_home: &Path) -> Self {
        Self {
            path: codex_home.join("policies/cowdex/ux_policy.md"),
        }
    }

    pub(crate) async fn install(&self) -> std::io::Result<()> {
        const POLICY: &str = include_str!("../../templates/ux_policy.md");
        if tokio::fs::read_to_string(&self.path).await.ok().as_deref() != Some(POLICY) {
            tokio::fs::create_dir_all(self.path.parent().expect("policy directory")).await?;
            tokio::fs::write(&self.path, POLICY).await?;
        }
        Ok(())
    }
}

impl ContextualUserFragment for UxPolicy {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<ux_policy>", "</ux_policy>")
    }

    fn body(&self) -> String {
        format!(
            "For UI/UX work other than focused bug fixes, read {} before designing, implementing, \
             or reviewing the interface and strictly follow it. The main agent must ensure \
             compliance across the delivered journey; the UX agent must enforce it within its \
             assigned scope. Keep copy concise, state each point once, and do not introduce \
             Material Design or flat themes. Bug fixes should preserve the established design \
             and verify the affected behavior. Do not read the policy for unrelated work.",
            self.path.display()
        )
    }
}

impl WorldStateSection for UxPolicy {
    const ID: &'static str = "ux_policy";
    type Snapshot = WorldStateHash;

    fn snapshot(&self) -> Self::Snapshot {
        WorldStateHash::from_fragment(self)
    }

    fn matches_legacy_fragment(role: &str, text: &str) -> bool {
        role == "developer" && Self::matches_text(text)
    }

    fn has_retained_fragment_matcher() -> bool {
        true
    }

    fn matches_retained_fragment(role: &str, text: &str) -> bool {
        Self::matches_legacy_fragment(role, text)
    }

    fn render_diff(
        &self,
        previous: PreviousSectionState<'_, Self::Snapshot>,
    ) -> Option<Box<dyn ContextualUserFragment>> {
        match previous {
            PreviousSectionState::Known(previous) if *previous == self.snapshot() => None,
            PreviousSectionState::Unknown => None,
            PreviousSectionState::Absent | PreviousSectionState::Known(_) => {
                Some(Box::new(self.clone()))
            }
        }
    }
}
