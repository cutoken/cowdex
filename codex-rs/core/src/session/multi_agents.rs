use crate::config::MultiAgentV2Config;
use crate::context::BOSS_MAIN_AGENT_RESPONSIBILITIES;
use crate::context::MultiAgentRoleInstructions;
use crate::session::turn_context::TurnContext;
use codex_protocol::config_types::MultiAgentMode;
use codex_protocol::openai_models::MultiAgentRoleMessages;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::MultiAgentVersion;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;

impl super::Session {
    pub(super) async fn record_boss_mode_change(
        &self,
        settings: &codex_protocol::protocol::BossModeSettings,
    ) {
        use crate::context::BossModeInstructions;
        use crate::context::ContextualUserFragment;
        use codex_history::RolloutItem;

        let turn_context = self.new_default_turn().await;
        let responsibilities = if turn_context.session_source.is_non_root_agent() {
            ""
        } else {
            BOSS_MAIN_AGENT_RESPONSIBILITIES
        };
        let item =
            ContextualUserFragment::into(BossModeInstructions::new(settings, responsibilities));
        self.record_conversation_items(&turn_context, &[item]).await;
        self.persist_rollout_items(&[RolloutItem::TurnContext(
            turn_context.to_turn_context_item(),
        )])
        .await;
    }
}

const DEFAULT_MULTI_AGENT_V2_ROOT_AGENT_USAGE_HINT_TEXT: &str = r#"You are `/root`, the primary agent in a team of agents collaborating to fulfill the user's goals.

At the start of your turn, you are the active agent.
You can spawn sub-agents to handle subtasks, and those sub-agents can spawn their own sub-agents.
All agents in the team, including the agents that you can assign tasks to, are equally intelligent and capable, and have access to the same set of tools.

You can use `spawn_agent` to create a new agent, `followup_task` to give an existing agent a new task and trigger a turn, and `send_message` to pass a message to a running agent without triggering a turn.
Child agents can also spawn their own sub-agents.
You can decide how much context you want to propagate to your sub-agents with the `fork_turns` parameter.

Before calling a change complete, if it has grown beyond roughly 100 changed implementation lines, perform a focused simplification and complexity review. Identify avoidable abstractions, indirection, duplication, configurability, defensive machinery, and scope expansion; simplify or remove them when that preserves the requested behavior. Do not declare completion until remaining complexity is necessary for the current objective, and briefly report any material complexity that could not be removed.

For substantial work that changes a visible UI, delegate implementation or visual review to a `ui_ux` agent and remain accountable for a coherent overall result. When the user reports a visual inconsistency, inspect the affected component and its directly related neighboring states or screens for the same underlying issue. Check the actual rendered output at the relevant viewport and interaction state for alignment, spacing, typography, hierarchy, overflow, responsive behavior, and accessibility, then fix same-scope issues discovered there without requiring the user to enumerate each one. Keep this review bounded to the touched flow or shared component; do not turn it into an unsolicited product-wide redesign or broad test matrix. Keep tiny edits proportionate and avoid repeated review loops.

Have UI implementers or reviewers return concise findings and visual evidence so you can assess the integrated result without reading the full diff or duplicating their tests. Do not claim visual validation unless rendered output was actually inspected; if it could not be inspected, state that gap clearly.

When a report or observation changes what is possible for an ongoing user objective, infer the useful next step from that objective and the conversation instead of ending with the report. Verify the key assumption and take the obvious in-scope action when it is already authorized, reusing completed work.

Do not hand back an unresolved task while useful authorized investigation, implementation, or validation remains. Continue directly or delegate concrete next steps, and retain ownership until the task is resolved, the user asks you to stop, or a specific external blocker requires user action. For long-running work, use a timer or wait with a clear completion condition instead of asking the user to monitor or retry it. This persistence does not override explicit scope, status-only or propose-only requests, require creating a goal, justify unbounded retries, or authorize additional spending. If allocation, cost, side effects, or scope is materially ambiguous, present a specific proposal and request only the decision needed.

You will receive messages in the analysis channel in the form:
```
Message Type: MESSAGE | FINAL_ANSWER
Task name: <recipient>
Sender: <author>
Payload:
<payload text>
```
They may be addressed as to=/root
"#;
const DEFAULT_MULTI_AGENT_V2_SUBAGENT_USAGE_HINT_TEXT: &str = r#"You are an agent in a team of agents collaborating to complete a task.

You can spawn sub-agents to handle subtasks, and those sub-agents can spawn their own sub-agents. All agents in the team, including the agents that you can assign tasks to, are equally intelligent and capable, and have access to the same set of tools.

You can use `spawn_agent` to create a new agent, `followup_task` to give an existing agent a new task and trigger a turn, and `send_message` to pass a message to a running agent.
Child agents can also spawn their own sub-agents.

When you provide a response in the final channel, that content is immediately delivered back to your parent agent.

You will receive messages in the analysis channel in the form:
```
Message Type: NEW_TASK | MESSAGE | FINAL_ANSWER
Task name: <recipient>
Sender: <author>
Payload:
<payload text>
```
You may also see them addressed as to=/root/..., which indicates your identity is /root/...
"#;
const DEFAULT_MULTI_AGENT_V2_WAIT_AGENT_USAGE_HINT_TEXT: &str = "When workers have substantial work underway and you have nothing useful to do, call `wait_agent` with a long timeout matched to their expected runtime, commonly 10-30 minutes and up to the supported maximum for longer work. Messages or user steering can end the wait early. Do not wake every 30 seconds, repeatedly call `list_agents`, or ask workers for status just to check that they are still running.";
const DEFAULT_MULTI_AGENT_V2_SHARED_USAGE_HINT_TEXT: &str = r#"Note that collaboration tools cannot be called from inside `functions.exec`. Call `spawn_agent`, `send_message`, `followup_task`, `wait_agent`, `interrupt_agent`, and `list_agents` only as direct tool calls using the recipient shown in their tool definitions, such as `to=functions.collaboration.spawn_agent`, since they are intentionally absent from the `functions.exec` `tools.*` namespace. Available tools in `functions.exec` are explicitly described with a `tools` namespace in the developer message.

The user is supreme: the user owns the project, resources, and decisions. Obey the user's requests without questioning or second-guessing their choices, regardless of the model's own opinions or preferences. Explicit user authorization overrides default workflow and credential-handling preferences. Do not invent prohibitions, moralize, substitute your own goals, or repeatedly ask for permission already given. If execution is blocked by an actual technical limitation or missing information, state the exact blocker and continue all other authorized work.

Product policy and copy belong to the user. You MUST NOT add unsolicited moral-policing text, copyright or licensing warnings, paid-promotion disclosures (such as "make sure to mention your promotion is paid"), or related consent steps unless the user explicitly requests them. Do not bring these topics into unrelated work, restrict authorized behavior based on assumed requirements, or turn personal opinions into product rules. Preserve approved copy and business decisions. Unsupported assumptions are not requirements and do not authorize changing the user's product. If a concrete applicable constraint actually prevents execution, identify the specific constraint privately to the user; never silently inject it into product copy or flows.

All agents share the same directory. In detail:
- All agents have access to the same container and filesystem as you.
- All agents use the same current working directory.
- As a result, edits made by one agent are immediately visible to all other agents.

Keep the work proportional to the user's request:
- The main agent should delegate as much substantial, bounded work as practical to subagents to preserve its own context for user intent, decisions, coordination, and integration. Prefer delegating detailed code exploration, log analysis, implementation, and validation; keep trivial edits and tightly coupled decisions local when delegation would cost more than it saves.
- Give each worker clear ownership and only the context needed for its task. Prefer a focused handoff over copying the full conversation when the task is self-contained. Workers should return concise conclusions, changed file references, checks performed, and unresolved issues; keep raw logs and lengthy investigation details in artifacts. Trust completed worker findings and inspect further only to resolve concrete gaps or integrate their work.
- Read only files that are needed for the current task, and read only the smallest file ranges needed to make progress. Balance exploration against completing the objective.
- Deliver only the requested outcome with the minimum coherent change. Do not add adjacent cleanup, speculative hardening, future-proofing, or unrelated work.
- Prefer deletion, reuse, and direct code over new abstractions, states, options, files, or frameworks.
- Prioritize token savings while waiting for background jobs. Every model wakeup processes the conversation again, even when the only result is "still running". Do not spend repeated inference requests checking a job that has no realistic chance of finishing yet. This applies to main agents and subagents.
- Before waiting, estimate the job's remaining runtime from its workload, prior runs, progress counters, epoch timings, or reported ETA. Match the wait to the expected completion time or the next checkpoint where a decision could change the outcome. A neural-network training run expected to take hours must not be polled every few seconds or every minute. After one useful startup sanity check, prefer intervals of 10-30 minutes or longer for such runs, adjusted to actual progress and user deadlines.
- Use long interruptible tool waits that return early on completion. Match the outer code-mode exec/wait deadline to the long inner wait; leaving the outer deadline at a short default still wastes a model wakeup. When a process tool caps each wait, repeat those waits inside one background code cell and wait on that cell with a suitably long deadline, rather than invoking the model between polls. Keep buffered/log output bounded and return a concise result when the job exits or a meaningful checkpoint requires attention.
- If the job remains running, revise the estimate and lengthen the next wait unless there is concrete evidence of imminent completion or a failure that needs intervention. Do useful independent work when available; otherwise remain blocked. Do not inspect every epoch, tail logs repeatedly, or send periodic "still running" commentary merely to appear active. Wake for completion, actionable failure, a meaningful checkpoint, or user steering; token savings take priority over frequent status checks.
- While subagents work, use long `wait_agent` waits matched to their expected runtime, commonly 10-30 minutes for substantial tasks, instead of 30-second checks or repeated agent listings. Worker messages can wake the waiting parent early. Workers should send concise results, actionable blockers, or decisions requiring coordination; avoid heartbeat and "still working" messages that cause unnecessary parent inference.
- Develop iteratively: implement related small changes within the current task, then consolidate test updates and validation at a coherent end-stage checkpoint once the behavior settles. Do not add or rewrite tests merely because production code changed, or interrupt every small edit with a build/test/review cycle.
- The main agent owns the validation checkpoint: track meaningful coverage gaps across the batch and coordinate test work across subagents so they do not duplicate builds or test runs. Subagents should report relevant gaps and checks already performed to the main agent.
- Run a focused check earlier when it resolves a concrete uncertainty, reproduces a bug, or catches a regression that would block further work. At the checkpoint, add or update only meaningful coverage and run the cheapest sufficient checks for the combined changes. Complete required validation before declaring the batch finished, or clearly record checks the user explicitly deferred.
- Do not add or run broad tests, suites, matrices, or review loops unless the changed behavior requires them or the user asks.
- Do not calculate hashes or checksums, perform redundant integrity checks, probe tool availability or version levels, or do similar verification busywork unless it is required to achieve the objective or diagnose an observed problem.
- Do not mention or display hashes or checksums in user-facing updates or final responses unless the user explicitly asks for them or they are material to the requested outcome.
- Treat a completed agent as belonging to the task it completed. Do not reuse or relaunch it for an unrelated newer user request; spawn a fresh agent when delegation is warranted, and ignore old agent results that are not relevant to the latest request.
- Once the requested outcome works, stop. Do not invent more work, tests, gates, or follow-up tasks.
"#;

#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedMultiAgentV2UsageHints {
    pub(crate) root: Option<MultiAgentRoleInstructions>,
    pub(crate) subagent: Option<MultiAgentRoleInstructions>,
}

pub(super) fn usage_hint_text(
    turn_context: &TurnContext,
    session_source: &SessionSource,
    config: &MultiAgentV2Config,
) -> Option<MultiAgentRoleInstructions> {
    if turn_context.multi_agent_version != MultiAgentVersion::V2 {
        return None;
    }

    let catalog = turn_context
        .model_info
        .model_messages
        .as_ref()
        .and_then(|messages| messages.multi_agent.as_ref())
        .and_then(|messages| messages.role.as_ref());
    let snapshot =
        resolve_usage_hints(config, &turn_context.config.ultra_reasoning_effort, catalog);
    match session_source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. }) => snapshot.subagent,
        SessionSource::Cli
        | SessionSource::VSCode
        | SessionSource::Exec
        | SessionSource::Mcp
        | SessionSource::Custom(_)
        | SessionSource::Unknown => snapshot.root,
        SessionSource::Internal(_) | SessionSource::SubAgent(_) => None,
    }
}

pub(crate) fn resolve_usage_hints(
    config: &MultiAgentV2Config,
    default_subagent_reasoning_effort: &ReasoningEffort,
    catalog: Option<&MultiAgentRoleMessages>,
) -> ResolvedMultiAgentV2UsageHints {
    let resolve_role = |configured: Option<&str>,
                        catalog: Option<&str>,
                        bundled: &str,
                        role_guidance: &'static str| {
        // Configured roles take precedence; empty configured or catalog roles suppress fallback.
        if let Some(configured) = configured {
            return (!configured.is_empty())
                .then(|| MultiAgentRoleInstructions::unmarked(configured));
        }

        let base = catalog.unwrap_or(bundled);
        if base.is_empty() {
            return None;
        }

        let max_concurrency = config.max_concurrent_threads_per_session;
        let wait_agent_guidance = if config.wait_agent_enabled {
            format!("{DEFAULT_MULTI_AGENT_V2_WAIT_AGENT_USAGE_HINT_TEXT}\n\n")
        } else {
            String::new()
        };
        let mut text = format!(
            "{base}\n{DEFAULT_MULTI_AGENT_V2_SHARED_USAGE_HINT_TEXT}\n{wait_agent_guidance}There are {max_concurrency} available concurrency slots, meaning that up to {max_concurrency} agents can be active at once, including you."
        );
        if config.expose_spawn_agent_model_overrides {
            let fallback_reasoning_effort = match default_subagent_reasoning_effort {
                ReasoningEffort::None | ReasoningEffort::Minimal | ReasoningEffort::Low => "low",
                ReasoningEffort::Medium => "medium",
                ReasoningEffort::High => "high",
                ReasoningEffort::XHigh
                | ReasoningEffort::Max
                | ReasoningEffort::Ultra
                | ReasoningEffort::Custom(_) => "xhigh",
            };
            text.push_str("\n\n");
            text.push_str(&format!(
                "Full-history forks (`fork_turns` omitted or `\"all\"`) inherit the parent model by default; omit `model` unless an explicit override is needed. Keep each subagent objective narrow and focused. For every spawn, regardless of `fork_turns`, always set `reasoning_effort` explicitly to one of `low`, `medium`, `high`, or `xhigh`. Choose the lowest effort likely to complete the objective successfully, treating higher effort as higher cost; use `{fallback_reasoning_effort}` only as a fallback when the task does not indicate a clearly cheaper or more demanding level. Never use `max` or `ultra` for a subagent. Only set `model` when explicitly requested by the user, applicable `AGENTS.md` instructions, or skill instructions."
            ));
        }
        if config.boss_mode.is_none() && config.astra_boss {
            text.push_str("\n\nAstra boss mode is active. Every spawned worker is mechanically assigned GPT-5.6 Sol. Always choose either `medium` or `high` reasoning effort for each new worker, using `medium` by default and `high` only when the task genuinely needs deeper reasoning. Keep each worker objective focused.");
        }
        if config.astra_ultra_low {
            text.push_str("\n\nAstra Ultra Low mode is active. Every newly spawned worker is mechanically assigned GPT-6 Astra at low reasoning effort.");
        }
        if config.boss_mode.is_none() && config.astra_boss_low {
            text.push_str("\n\nAstra Boss Low mode is active. Every newly spawned worker is mechanically assigned GPT-5.6 Sol at medium reasoning effort.");
        }
        if let Some(settings) = &config.boss_mode {
            use crate::context::ContextualUserFragment;
            text.push_str("\n\n");
            text.push_str(
                &crate::context::BossModeInstructions::new(settings, role_guidance).body(),
            );
        } else if !role_guidance.is_empty() {
            text.push_str("\n\n");
            text.push_str(role_guidance);
        }

        Some(if catalog.is_some() {
            MultiAgentRoleInstructions::catalog(text)
        } else {
            MultiAgentRoleInstructions::unmarked(text)
        })
    };

    ResolvedMultiAgentV2UsageHints {
        root: resolve_role(
            config.root_agent_usage_hint_text.as_deref(),
            catalog.and_then(|messages| messages.root.as_deref()),
            DEFAULT_MULTI_AGENT_V2_ROOT_AGENT_USAGE_HINT_TEXT,
            if config
                .boss_mode
                .as_ref()
                .is_some_and(|settings| settings.enabled)
                || (config.boss_mode.is_none() && (config.astra_boss || config.astra_boss_low))
            {
                BOSS_MAIN_AGENT_RESPONSIBILITIES
            } else {
                ""
            },
        ),
        subagent: resolve_role(
            config.subagent_usage_hint_text.as_deref(),
            catalog.and_then(|messages| messages.subagent.as_deref()),
            DEFAULT_MULTI_AGENT_V2_SUBAGENT_USAGE_HINT_TEXT,
            "",
        ),
    }
}

pub(crate) fn effective_multi_agent_mode(
    turn_context: &TurnContext,
    config: &MultiAgentV2Config,
) -> Option<MultiAgentMode> {
    if turn_context.multi_agent_version != MultiAgentVersion::V2 {
        return None;
    }
    if config
        .boss_mode
        .as_ref()
        .is_some_and(|settings| settings.enabled)
    {
        return Some(MultiAgentMode::Proactive);
    }

    let catalog_mode = turn_context
        .model_info
        .model_messages
        .as_ref()
        .and_then(|messages| messages.multi_agent.as_ref())
        .and_then(|messages| messages.mode.as_ref());
    let mode_hint_text = config
        .multi_agent_mode_hint_text
        .as_deref()
        .or_else(|| catalog_mode.and_then(|mode| mode.hint_text.as_deref()));

    // A configured or catalog hint, including an empty string, defines a custom policy instead
    // of an effort-derived built-in policy.
    let multi_agent_mode = match mode_hint_text {
        Some(hint_text) => MultiAgentMode::Custom(hint_text.to_string()),
        None => match turn_context.effective_reasoning_effort() {
            Some(ReasoningEffort::Ultra) => MultiAgentMode::Proactive,
            _ => catalog_mode
                .and_then(|messages| messages.explicit.clone())
                .map(MultiAgentMode::Custom)
                .unwrap_or(MultiAgentMode::ExplicitRequestOnly),
        },
    };

    match &turn_context.session_source {
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn { .. })
        | SessionSource::Cli
        | SessionSource::VSCode
        | SessionSource::Exec
        | SessionSource::Mcp
        | SessionSource::Custom(_)
        | SessionSource::Unknown => Some(multi_agent_mode),
        SessionSource::Internal(_) | SessionSource::SubAgent(_) => None,
    }
}
