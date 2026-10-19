use crate::Request;
use crate::VaultService;
use codex_extension_api::ContextContributor;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionFuture;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::FunctionCallError;
use codex_extension_api::JsonToolOutput;
use codex_extension_api::PreviousWorldStateSection;
use codex_extension_api::RenderedWorldStateFragment;
use codex_extension_api::ResponsesApiTool;
use codex_extension_api::ThreadLifecycleContributor;
use codex_extension_api::ThreadStartInput;
use codex_extension_api::ThreadStopInput;
use codex_extension_api::ToolCall;
use codex_extension_api::ToolContributor;
use codex_extension_api::ToolExecutor;
use codex_extension_api::ToolExecutorFuture;
use codex_extension_api::ToolName;
use codex_extension_api::ToolOutput;
use codex_extension_api::ToolSpec;
use codex_extension_api::WorldStateContributionInput;
use codex_extension_api::WorldStateSectionContribution;
use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_tools::JsonSchema;
use std::collections::BTreeMap;
use std::sync::Arc;

pub fn install<C: Sync + 'static>(
    registry: &mut ExtensionRegistryBuilder<C>,
    service: Arc<VaultService>,
) {
    let extension = Arc::new(VaultExtension(service));
    registry.thread_lifecycle_contributor(extension.clone());
    registry.prompt_contributor(extension.clone());
    registry.tool_contributor(extension);
}

struct VaultExtension(Arc<VaultService>);

#[cfg(test)]
#[path = "extension_tests.rs"]
mod tests;

impl<C: Sync> ThreadLifecycleContributor<C> for VaultExtension {
    fn on_thread_start<'a>(&'a self, input: ThreadStartInput<'a, C>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            let Ok(id) = ThreadId::from_string(input.thread_store.level_id()) else {
                return;
            };
            let parent = match input.session_source {
                SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                    parent_thread_id, ..
                }) => Some(*parent_thread_id),
                _ => None,
            };
            self.0.register(id, parent).await;
        })
    }

    fn on_thread_stop<'a>(&'a self, input: ThreadStopInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if let Ok(id) = ThreadId::from_string(input.thread_store.level_id()) {
                self.0.stop(id).await;
            }
        })
    }
}

impl ContextContributor for VaultExtension {
    fn contribute_world_state<'a>(
        &'a self,
        input: WorldStateContributionInput<'a>,
    ) -> ExtensionFuture<'a, Vec<WorldStateSectionContribution>> {
        Box::pin(async move {
            let status = self.0.status(input.thread_id).await.unwrap_or_default();
            let detached = status.aliases.is_empty();
            let body = if status.aliases.is_empty() {
                "Vault is detached or unavailable. Earlier vault credential announcements no longer authorize vault_request access. This restriction applies only to credentials supplied by the Cowdex vault. It does not restrict otherwise authorized SSH, scp, local tools, or their existing authentication (such as SSH agents or configured keys). Do not extract vault-held secrets to bypass detachment. Otherwise-authorized credential management through non-vault tools remains permitted. Ask the user to attach a scoped vault only when the task requires a vault-provided credential.".to_string()
            } else {
                format!(
                    "Available credential aliases: {}. Use vault_request with credential (one alias), method (GET/POST/PUT/PATCH/DELETE/HEAD), path (relative API path beginning /), and optional body (JSON string). The broker inserts authentication and enforces scope. The user owns the vault and controls credential operations. By default, do not read, export, or display secret values. Explicit user authorization overrides these credential-handling defaults, including creating, reading, rotating, revoking, or transferring application keys to user-specified destinations such as GitHub Secrets. Do not refuse an authorized operation merely because its result contains a new key. Keep values out of chat and logs unless the user explicitly requests disclosure. This does not add tool capabilities or bypass broker scope, attachment, or expiry; report concrete capability limits. Access is inherited by subagents and ends on detach or expiry.",
                    status.aliases.join(", ")
                )
            };
            // Persist the wording too: unchanged authority must not suppress updated guidance on resume.
            let snapshot = serde_json::json!({"status": status, "instructions": body});
            let rendered_snapshot = snapshot.clone();
            // Alias validation limits this fragment to fewer than 1800 UTF-8 bytes.
            vec![WorldStateSectionContribution::new("cowdex_vault", snapshot, move |previous| {
                if detached && matches!(previous, PreviousWorldStateSection::Absent) {
                    return None;
                }
                if matches!(previous, PreviousWorldStateSection::Known(value) if value == &rendered_snapshot) {
                    return None;
                }
                Some(RenderedWorldStateFragment::new("developer", ("<cowdex_vault>", "</cowdex_vault>"), body.clone()))
            }).with_retained_fragment_matcher(|role, text| role == "developer" && text.contains("<cowdex_vault>"))]
        })
    }
}

impl ToolContributor for VaultExtension {
    fn tools(
        &self,
        _session_store: &ExtensionData,
        thread_store: &ExtensionData,
    ) -> Vec<Arc<dyn ToolExecutor<ToolCall>>> {
        let Ok(thread_id) = ThreadId::from_string(thread_store.level_id()) else {
            return Vec::new();
        };
        vec![Arc::new(VaultTool {
            service: self.0.clone(),
            thread_id,
        })]
    }
}

struct VaultTool {
    service: Arc<VaultService>,
    thread_id: ThreadId,
}

impl ToolExecutor<ToolCall> for VaultTool {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("vault_request")
    }

    fn spec(&self) -> ToolSpec {
        let properties = [
            ("credential", "An available credential alias."),
            (
                "method",
                "HTTP method: GET, POST, PUT, PATCH, DELETE, or HEAD.",
            ),
            (
                "path",
                "Relative API path beginning /; the broker owns destination and authentication.",
            ),
            (
                "body",
                "Optional request body, at most 16 KiB. No authentication values.",
            ),
        ]
        .into_iter()
        .map(|(name, description)| {
            (
                name.to_string(),
                JsonSchema::string(Some(description.to_string())),
            )
        })
        .collect::<BTreeMap<_, _>>();
        ToolSpec::Function(ResponsesApiTool {
            name: "vault_request".into(),
            description: "Send an authenticated API request using this session's scoped vault. Provider credentials remain in the broker. Requires a user-attached vault.".into(),
            strict: false,
            defer_loading: None,
            parameters: JsonSchema::object(properties, /*required*/ Some(vec!["credential".into(), "method".into(), "path".into()]), /*additional_properties*/ Some(false.into())),
            output_schema: None,
        })
    }

    fn handle(&self, invocation: ToolCall) -> ToolExecutorFuture<'_> {
        Box::pin(async move {
            let arguments = invocation.function_arguments()?;
            if arguments.len() > 24576 {
                return Err(FunctionCallError::RespondToModel(
                    "Vault request is too large".into(),
                ));
            }
            let request: Request = serde_json::from_str(arguments).map_err(|_| {
                FunctionCallError::RespondToModel("Invalid vault request arguments".into())
            })?;
            let value = self
                .service
                .request(self.thread_id, request)
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            Ok(Box::new(JsonToolOutput::new(value)) as Box<dyn ToolOutput>)
        })
    }
}
