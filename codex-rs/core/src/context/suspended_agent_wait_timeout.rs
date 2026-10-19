use super::ContextualUserFragment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SuspendedAgentWaitTimeout;

impl ContextualUserFragment for SuspendedAgentWaitTimeout {
    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<automatic_wait_timeout>", "</automatic_wait_timeout>")
    }

    fn body(&self) -> String {
        "The automatically resumed wait reached its original deadline. Reassess the outstanding agents once. Do not automatically wait again unless the task still requires it."
            .to_string()
    }
}
