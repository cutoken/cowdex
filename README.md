# Cowdex

## Install

```sh
curl -fsSL https://github.com/cutoken/cowdex/releases/latest/download/install.py | python3 - --repo cutoken/cowdex
export PATH="$HOME/.local/bin:$PATH"
cowdex login
cowdex
```

Requires Python 3.10+ and curl on Linux/macOS.
Add `~/.local/bin` to your shell profile to keep the command on your PATH.

**A coding agent for work that spans agents, sessions and providers.**

Cowdex builds on OpenAI Codex and adds controls for sustained engineering work:
a lead agent that can coordinate a team, resumable work across providers, and
explicit choices about models, concurrency, context and credentials.

## One lead. A team when you need it.

Ultra mode lets the lead agent delegate bounded tasks to workers, reviewers and
UI/UX specialists, then bring their results together. Workers receive a defined
task and role, then send concise reports back to the lead.

Boss mode lets you choose worker models and reasoning effort independently of
the lead. Use smaller workers for execution or keep the team on the same model.

```sh
cowdex --astra-ultra
```

## Your work should outlast a model choice.

Start with OpenAI, use a Z.ai Coding Plan, or choose DeepSeek through OpenRouter.
Resume an existing conversation with another provider without starting the
project's history over.

```sh
cowdex --zai
cowdex resume --openrouter
cowdex --zai --astra-ultra
```

Explicit provider flags take precedence over saved session settings. In Z.ai
and OpenRouter modes, newly spawned agents use the selected provider and model.
Configure `ZAI_API_KEY` or `OPENROUTER_API_KEY`, or use credentials from `~/.cando/`.

## Context is part of the engineering.

Delegation only helps when the agent receives useful context. Cowdex can preserve
the parent's prompt prefix when forking workers so the provider can reuse cached
input. Idle workers can be unloaded and brought back with their queued messages
and environment state.

Compaction marks retained user messages with their age, helping the agent
distinguish an old request from a new instruction. Larger main-agent windows and
optional lower-cost compaction give you further control; actual context limits
depend on the model and provider.

## Long jobs need a way to keep going.

Timers and wake-ups let agents wait for useful checkpoints. Transient server
errors retry every 60 seconds; WebSocket failures can fall back to HTTPS.
An external login change is picked up on the next user message, so switching
accounts does not require closing the conversation.

You keep direct controls in the terminal:

| Command           | What you control                                               |
| ----------------- | -------------------------------------------------------------- |
| `/model`          | Model, reasoning effort and normal/Ultra mode                  |
| `/boss`           | The model and effort used for future workers                   |
| `/limit_agents N` | Session concurrency, including the main agent                  |
| `/goal`           | The active goal and its lifecycle                              |
| `/vault`          | Attach or revoke access to scoped credentials                  |
| `/cleanup`        | Reclaim old subagent sessions, logs and excess rollout history |

Cleanup protects running sessions and retains the context needed to resume
trimmed conversations, with originals moved to backup.

## Credentials belong to you.

The optional Linux vault manager keeps provider credentials in a local broker
and gives agents scoped API access. Attach a vault while the model is working,
choose what it can reach, and revoke that access when the job is done.

```sh
cowdex-vault
```

Cowdex also bundles a UX policy that agents consult for UI work, emphasizing a
clear user journey, concise copy and consistent interaction design.

## Familiar tools. A separate installation.

Cowdex retains Codex's file editing, command execution, MCP integration and
app-server interface. It installs beside regular Codex, with its own matching
code-mode host, ripgrep, shell and platform sandbox helpers.

Your own ChatGPT subscription or provider API account supplies model access.
Settings and sessions use `CODEX_HOME` (default `~/.codex`); set it explicitly if
you want separate state from regular Codex. Updates preserve older installed
releases and leave running sessions intact.

<details>
<summary>Other installation options and platform notes</summary>

Download and extract a release archive, then run `python3 install.py`.
On Windows, use `py install.py`. To install a downloaded archive directly:

```sh
python3 scripts/install_cowdex.py --archive /path/to/cowdex-package.tar.gz
```

Pass `--release VERSION` to select a published version, or `--package-root` and
`--bin-dir` to choose installation locations.

Release packages cover Linux x64/ARM64, macOS Intel/Apple Silicon, and Windows
x64/ARM64. Each platform passes bundled code-mode and installation smoke checks
on GitHub-hosted runners. Mac packages omit Developer ID signing and notarization.

The vault manager currently supports Linux only. It requires Python, venv/pip
and installs its Python dependency on first use.

</details>

<details>
<summary>Build from source</summary>

Use Rust 1.95.0 and Python 3.11+ on the target platform:

```sh
python3 scripts/build_codex_package.py --variant cowdex --cargo-profile release \
  --package-dir dist/package --archive-output dist/cowdex-package.tar.gz
```

The builder compiles first-party binaries together and downloads pinned V8
libraries/bindings and prebuilt ripgrep/shell dependencies. Linux musl toolchain
setup and the six-platform build matrix are in
[the release workflow](.github/workflows/cowdex-release.yml).
Windows packages use `.zip` archives.

</details>
