# 🐮 Cowdex

Cowdex is a Codex fork built to save tokens, with a slower release cycle to avoid chasing daily upstream regressions.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/cutoken/cowdex/main/scripts/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
cowdex login
cowdex
```

On Windows:

```powershell
irm https://raw.githubusercontent.com/cutoken/cowdex/main/scripts/install.ps1 | iex
```

Run `cowdex update` to update. Cowdex also prompts when a new release is available.

PS: Cowdex works great with [AgentCron project management software](https://agentcron.ai).

```sh
cowdex mcp add agentcron --url https://api.agentcron.ai/api/mcp
cowdex mcp login agentcron
```

Cowdex can then see your boards and files on AgentCron.

## Features

- **Worker events:** Stop wasting tokens on repeated status checks.
- **Boss mode:** Keep a strong main agent and use cheaper workers.
- **Ultra effort:** Use Ultra without being forced to max effort.
- **Engineering guardrails:** Less overengineering, test bloat and unrelated work.
- **Automatic retries:** Less babysitting after “Selected model is at capacity” and other transient errors.
- **Token counts:** See cached input, reasoning, output and total token usage live.
- **Lighter storage:** Less SSD churn from logs and oversized session files.
- **Keepwarm:** Keep extra accounts warm without interrupting your current login.
- **Vault:** Let agents use API keys without pasting them into chats.
- **Provider switching:** Continue the same conversation across providers.

## New functionality

| Command                       | What it does                                                                             |
| ----------------------------- | ---------------------------------------------------------------------------------------- |
| `cowdex --ultra-effort high`  | Run Ultra at your chosen effort: low, medium, high, xhigh or max, where supported.       |
| `cowdex --worker sol:medium`  | Fix the worker model and effort; bare `--worker` defaults to Sol/medium.                 |
| `/limit_agents 8`             | Set this session's concurrent agent limit, including the main agent; omit the number to show it. |
| `cowdex --effort high`        | Set the main agent's reasoning effort.                                                   |
| `cowdex --max-context`        | Use the main model's maximum supported context window.                                   |
| `cowdex --compact-with-lower` | Use cheaper compaction for Astra and Sol at high effort or above.                        |
| `cowdex --openrouter`         | Work through your OpenRouter account.                                                    |
| `cowdex --zai`                | Work through your Z.ai Coding Plan.                                                      |
| `cowdex keepwarm`             | Sign into another account and keep it active.                                            |
| `cowdex cleanup`              | Shrink session files and clear stale subagents and logs while preserving resume support. |
| `cowdex update`               | Install the latest Cowdex release.                                                       |
| `cowdex-vault`                | Open the credential vault manager.                                                       |

## Use OpenRouter or Z.ai

Bring your OpenRouter API account or Z.ai Coding Plan for the main agent and its workers. Set `OPENROUTER_API_KEY` or `ZAI_API_KEY`, then start with `--openrouter` or `--zai`. Use the same flag with `cowdex resume` to continue an existing conversation.

Keep your main model and choose workers separately with `--worker zai:glm-5.3` or `--worker openrouter:deepseek/deepseek-v4.1-flash:high`. The final effort is optional and defaults to medium; provider keys can also be saved with `cowdex login --provider zai --with-api-key` (or `openrouter`).

Change worker defaults during a session with `/worker <model> [effort]`.

## Vault

The vault is a self-contained binary with no Python dependency. Run `cowdex-vault`, add your keys, then use `/vault attach` inside Cowdex. [Vault setup](cowdex-vault/README.md).

Codex Apps are off by default; enable them with `cowdex --enable apps`.

[Build from source](scripts/codex_package/README.md).
