# Cowdex Vault prototype

A small Linux credential broker with an encrypted file and a human-unlocked,
absolute process lifetime. This is an initial integration prototype, not an
audited vault. Python 3.10+ is required; the browser launcher installs its Python
dependency automatically.

## Browser workflow

From the Cowdex repository, run:

```sh
./cowdex-vault/start
```

The command prepares its Python environment on first use, starts the manager in
the background, and opens **http://127.0.0.1:2749**. Running it again reopens the
manager. On first use, choose and confirm a vault password; subsequently, enter
that password to unlock. No filenames or JSON are needed.

If installed locally, simply run `cowdex-vault` from any directory.

Choose **Add credential**, select a provider, and paste the requested secret.
Templates fill in the API address and authentication format. **All API operations**
is the default; it allows every supported method and path on that provider's API
origin, within the permissions of your provider-issued key. Choose **Read only**
for GET/HEAD, or **Custom** for individual methods and a path restriction.
Every saved credential can be requested by Cowdex, up to eight entries per scope.
Previously unshared entries also become available when unlocked in this version.
Older vaults with more than eight stored entries remain editable; default attachment
uses the first eight aliases alphabetically. Use `--scope` to choose other entries.

| Template | Credentials to enter |
| --- | --- |
| [Cloudflare](https://developers.cloudflare.com/fundamentals/api/how-to/make-api-calls/) | API token (not a Global API Key) |
| [DigitalOcean](https://docs.digitalocean.com/reference/api/reference/public-apis/) | Personal access token |
| [Hetzner Cloud](https://docs.hetzner.cloud/reference/cloud) | Console project API token |
| [Hetzner Storage Box management](https://docs.hetzner.cloud/reference/hetzner) | Console API token |
| [Hetzner Robot / dedicated servers](https://robot.hetzner.com/doc/webservice/en.html) | Separate webservice username and password |
| [Twilio API key or account token](https://www.twilio.com/docs/usage/requests-to-twilio) | API key SID + secret, or Account SID + Auth Token |
| [GitHub](https://docs.github.com/en/rest/authentication/authenticating-to-the-rest-api) | Personal access token |
| [Stripe API v1](https://docs.stripe.com/api/authentication) | Secret or restricted API key |
| [Mailgun US / EU](https://documentation.mailgun.com/docs/mailgun/api-reference/mg-auth) | Account or domain sending API key; username is filled automatically |
| [MoonPay Commerce / Helio](https://docs.hel.io/reference/paylink/list-transactions) | Public API key + secret API key; production and Devnet presets |

**Custom API** supports bearer tokens or Basic authentication, with JSON or form
request bodies. Storage Box management controls boxes and subaccounts; it does
not provide file access or Object Storage/S3 signing. Twilio presets cover the
main `api.twilio.com` REST API. Stripe's preset covers v1 form-encoded endpoints.
Mailgun supports multipart text fields, including repeated recipients; file uploads
are not supported. MoonPay Commerce uses `api.hel.io` (or `api.dev.hel.io`): the
broker supplies its public `apiKey` query parameter and secret Bearer header. Agents
need only provide the API path, including `/v1`, and optional JSON body.

Save and return to an idle Cowdex session:

```text
/vault attach
```

Cowdex displays a public pairing code. In the unlocked vault page, compare that code
and the requested credentials, then click **Approve for 24 hours**. Only approve a
request you initiated. Repeat the same `/vault attach` command, including any scope
or profile options, to finish. You can
now close the page. Requests expire after five minutes; approved access expires 24
hours after approval or when the vault locks, whichever comes first. Reattach and
approve again when access expires. **Deny** refuses an unwanted request.

The private claim token and resulting session token are held only in Cowdex/broker
memory and never displayed in the browser, model context, or grant file. Reading
`grant.json` permits requesting approval, not making authenticated API calls.
The public pairing code cannot claim a grant. This approval boundary still does not
protect against malware that can inspect or modify Cowdex, the browser, or the broker.

This uses all saved credentials. To attach only some of them,
use `/vault attach --scope cf`. Parent sessions and subagents share
that attachment. `/vault detach` revokes that session family's access; the web
manager can continue serving other attached sessions.
After adding credentials, run `/vault attach` again to refresh the session and
its subagents, and approve the new request. The new attachment replaces the old one only after validation;
a failed refresh keeps the previous attachment.

**Edit** lets you replace a token or change its restrictions. A blank token field
keeps the saved token; existing token values are never sent back to the browser.
The same blank-to-keep behavior applies to saved Basic usernames. Token replacements
apply to existing sessions. Changing a credential's policy or Basic username
revokes affected attachments; attach again for the new policy. Changing provider
or authentication format requires fresh credentials. Editing an existing restricted
credential preserves its restrictions unless you explicitly change access.

The default encrypted file is `~/.local/share/cowdex-vault/vault.enc`. The manager
keeps access unlocked for 24 hours after the first unlock, even after closing
the page. **Lock vault** revokes all attachments. Locking or expiry
replaces the unlocked process with a fresh locked manager while retaining its
listening port. Reopening or refreshing the page requires your password again.
Use `./cowdex-vault/start --ttl 3600` for a one-hour window when starting a new
manager; the maximum is 24 hours. Browser authentication stays in page memory.

Advanced: `COWDEX_VAULT_HOME` selects an absolute default directory; set it identically
for the launcher and Cowdex's app-server. The default browser workflow runs as
your current OS user. The separate-account isolation requirement below still
applies when protecting against malicious processes with that user's privileges.

## Explicit terminal setup

For these lower-level commands, install `pip install -r requirements.txt` first.
This separate terminal workflow authorizes access directly through the human's
terminal and does not use browser approval; the 24-hour approval flow above applies
to the browser manager.

Create a private directory owned by the broker's OS user. All paths must be
absolute; existing socket and grant files are refused. Run setup from a human
terminal. Passwords and tokens are read with `getpass`, never command arguments.

```sh
python3 broker.py init --vault /private/vault.enc
python3 broker.py add --vault /private/vault.enc --alias cf \
  --origin https://api.cloudflare.com --methods GET,POST \
  --prefix /client/v4/accounts/YOUR_ACCOUNT
python3 broker.py serve --vault /private/vault.enc --scope cf \
  --socket /private/runtime/vault.sock --grant-file /private/runtime/grant.json \
  --ttl 7200
```

`add` also replaces a credential under an existing alias. Active brokers reload
token values before each authenticated request while keeping their original
origins, methods, path policies, and alias scope frozen. Repurposing a selected
alias to a different origin blocks requests until a new grant is created;
the replacement token is never sent to the old origin. Other policy changes
require a new grant to take effect. Vault encryption uses scrypt and AES-256-GCM. Only
selected aliases remain in the broker's live credential mapping after unlock;
Python does not promise erasure of previous allocation copies. Core dumps and
Linux process dumpability are disabled. Memory is not locked against swap: use
encrypted swap or disable swap on the broker host. The deadline destroys the
broker process even if a request is blocked; this bounds unlocked process
lifetime rather than promising Python memory zeroization. Maximum TTL is 24 hours.

The grant profile contains `{ "socket": "/absolute/path", "ticket": "..." }`.
It is mode 0600 and contains temporary broker authority, not API credentials.
Cowdex consumes it through the trusted attachment command. One broker serves one
preselected scope for a parent session and its subagents. The attach scope must
exactly match the broker scope; announcements cannot narrow actual permissions.
The ticket is single use. Detachment revokes the shared session and exits.

In an idle Cowdex session, attach the transferred profile using its absolute path:

```text
/vault attach /home/you/.cowdex/cf-grant.json --scope cf
/vault status
/vault detach
```

Cowdex announces only the granted aliases and the `vault_request` calling
convention. Existing and new subagents inherit the session's current access.
Resumed sessions start detached. The first version uses the request tool;
ordinary shell commands and SDKs do not receive credentials automatically.

For protection from agent-run processes, run the broker as a separate OS user
that Cowdex cannot impersonate or inspect, with broker code installed somewhere
the agent cannot modify. The terminal broker's `--socket-group GROUP` grants socket
access to a dedicated group; the socket directory also needs group traversal.
The broker-owned parent directories must not be writable by that group. Transfer
the grant profile to the Cowdex user through a trusted human channel with mode
0600; do not copy the encrypted vault or password. The broker deletes its original
profile on exit, so remove any transferred copy after attachment. A same-user
deployment does not provide this isolation. Root/full-host compromise is outside
this boundary.

The newline-delimited JSON socket protocol supports `attach`, `status`, `request`,
and `detach`. Requests are capped at 64 KiB. Authenticated requests supply only
an alias, allowed method, canonical relative path, and optional JSON body string.
The broker fixes the HTTPS origin and authentication header. For form-based
providers, pass a JSON object as the tool's body string: the broker converts its
scalar fields and lists to form parameters. Use bracketed field names for nested
Stripe parameters, for example `{"metadata[order_id]":"123"}`. It verifies TLS, refuses
redirects, rejects encoded path segments and oversized/compressed responses,
and returns no upstream headers. Responses are limited to 8192 bytes and exact
credential echoes are redacted. An upstream can transform a credential to evade
literal redaction: use trusted providers and carefully restrict allowed paths.
Broker scope controls credential use, not whether an authorized API action is wise.

Run `python3 -m unittest -v test_broker.py test_web.py` here. Tests use only fake credentials
and mocked HTTPS; no live model inference or provider requests are needed.
