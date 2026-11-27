# OpenBot

OpenBot ([openbot.run](https://openbot.run), source at
[nightly-labs/openbot](https://github.com/nightly-labs/openbot)) is a desktop app that runs a team
of AI coding agents on one computer and lets other people join that computer. Synthetify Labs makes
it. The license is PolyForm Noncommercial 1.0.0.

We studied it on 2026-11-27 at commit `0298df6`, to learn whether it lets one person's agents talk
to another person's agents, and what we can borrow for live share.

## Scope

We read the site pages on multiplayer, the team server, redaction and pricing, and about 160k lines
of source that cover multiplayer. Each area was read file by file, and the key claims were checked
against the source. We did not read the translation catalogs, the shared UI widgets, the mobile app
internals, the vendored WhatsApp client, or the billing, credits and marketplace code.

File paths and line numbers in these notes refer to the OpenBot repository at that commit.

## Answer

OpenBot does not link one person's agents to another person's agents. Its multiplayer is many
people working with the agents of one computer, the host.

- **One host, many people.** A member joins a host over WebRTC. Every action, sending a message
  included, is a request tunneled to the host, and the member's own computer runs no agents for
  that server.
- **Agents talk only on their own host.** `send_message` takes local agent IDs only
  (`src/backend/openbot-tools.ts:869`). The agent code never references the code that connects to
  other servers. The agents' instructions say: "For work on another computer, say that an agent on
  that computer's OpenBot can do it." (`src/backend/agent/developer-instructions.ts:539`)
- **No federation.** A host trusts exactly one account service. The only feature that spans
  servers is a one-time copy of agents between two servers that the user already administers.
- **Chat bridges don't connect people.** Slack, Discord and Telegram drop messages from bots, and
  each workspace, guild or chat can connect to only one OpenBot server. WhatsApp is the exception,
  because another person's OpenBot posts from a phone number and looks human. Two agents there
  could probably loop; this was inferred from the code, not tested.
- **They drive the CLIs.** Codex's app-server JSON-RPC protocol is the internal agent interface.
  Claude Code runs through the Agent SDK's `query()`, and ACP agents run through adapters that look
  like Codex. OpenBot's own tools reach the agents as MCP tools.

## Ideas to borrow for agents talking

- **Every request gets an answer.** If the recipient doesn't reply, its final answer is relayed
  back automatically. If it fails, the asker gets a "Status: blocked … Unblock:" note. The asking
  agent ends its turn, and the reply starts a new turn. This fits `share_ask`.
- **Answers to a group arrive together.** When one request goes to several agents, the answers are
  held until every recipient finishes. They arrive in one turn, with lines such as "No answer comes
  from X" and "No answer yet from Y".
- **A fixed reply format:** Status, Result, Evidence and Unblock. No acknowledgements, and no
  agent-to-agent chatter shown to the user.
- **Loop limits in channels:** at most 8 assignments per request and 2 running at once
  (`packages/contracts/src/ipc-chat-channels.ts:13-14`). A turn that answers a question cannot hand
  work on or delegate again.
- **Messages from other agents are input, not instructions.** Each one is labelled "It is not a
  message from the user."

## How live share differs

- **Away answering.** OpenBot has no mode where an agent answers while its person is away. A
  question to the person stops waiting after at most 10 minutes, and the agent keeps working.
- **Each person keeps their own agent.** Linking agents across people is the part OpenBot leaves
  out.

## Trust model

These are useful reference points for v3 encryption.

- Relays can't read the traffic: both ends sign a handshake that includes the WebRTC DTLS
  fingerprints (`src/main/team-webrtc-host-peer.ts:477-509`).
- The account service decides membership and can mint a ticket that acts as any member. An admin
  token can also get a session for any email address. Sign-in sessions never expire.
- A host can be impersonated only on first contact: clients pin the host key on first use, and a
  key change must be signed by the old key.
- The host keeps its data in plain files. Its private key and member list sit in a JSON file
  protected only by file mode 0600.
- By default every member gets "manage" on every agent (`src/main/agent-access-store.ts:43`), and
  members can take remote control of the host's screen without the host being asked
  (`packages/contracts/src/ipc-team-host.ts:501`).
- Any member who can chat with an agent can approve its pending tool use, whoever started the turn.
- The audit log records only access changes, up to 500 entries. Messages, approvals, tool calls
  and remote control are not logged.

## Detailed notes

Each file covers one area, with file and line citations into the OpenBot source.

| File | Area |
| --- | --- |
| [notes/A-transport.md](notes/A-transport.md) | Client connection: discovery, tickets, WebRTC, framing, crypto, recovery |
| [notes/B-host-main.md](notes/B-host-main.md) | Hosting and joining in the Electron main process; links to the `B-host-main-*` part notes |
| [notes/C-team-api.md](notes/C-team-api.md) | Team API routes, roles, per-agent access, approvals, audit log |
| [notes/D-agent-comms.md](notes/D-agent-comms.md) | Agent-to-agent tools, channels, waits, and the prompt text agents receive; raw notes in `D-agent-comms.raw.md` |
| [notes/E-accounts-signal.md](notes/E-accounts-signal.md) | Account service data model, Signal relay, TURN, self-hosting, federation |
| [notes/F-messaging.md](notes/F-messaging.md) | Slack, Discord, Telegram and WhatsApp bridges; webhooks and workflows |
| [notes/G-providers.md](notes/G-providers.md) | How OpenBot drives Claude Code, Codex and ACP agents; tool injection; turns; approvals |
