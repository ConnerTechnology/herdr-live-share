# C. OpenBot host Team API routes and member permission model

Source: `openbot-src/openbot` (untrusted clone, read only). Paths below are repo-relative. "Code" means a
source file I read; "Doc" means `docs/architecture/*.md`. Nothing was run.

## 0. The model in one paragraph

A "server" is one person's desktop (the host). Everything (agents, chats, threads, channels, browser,
files, vault, MCP servers, provider sign-ins) lives on the host. A remote person is a **member** of that
host with a `TeamRole` of `owner | admin | member` (`packages/contracts/src/ipc-team-host.ts:18-22`). Every
Team API request is authenticated to one member and routed through ~150 `route-*.ts` handlers in
`src/main/team-api/`, dispatched in order by `TeamApiServer.#handle` (`src/main/team-api-server.ts:1252-1563`).
Owners and admins are equivalent for almost everything (`canAdministerServer`, `ipc-team-host.ts:20`);
a few things are owner-only. For plain `member`s there is a second, per-agent layer: an **agent access
level** `none < view < chat < manage` (`packages/contracts/src/ipc-agent-access.ts:4`), set per agent for
"everyone" and for custom named **AgentAccessRoles** (groups of member IDs). The remote member does not
bring agents: they drive the host's agents. Agents never cross hosts.

### Agent access level resolution (code)

`src/main/agent-access-store.ts:36-51`:

```ts
if (member.role !== "member") return "manage";      // owner/admin always manage every agent
if (state === "closed") return "none";               // damaged file => members locked out
let level = state.everyone[agentId] ?? "manage";     // DEFAULT for an unlisted agent is MANAGE
for (const role of state.roles) { ... take max(level, role.agents[agentId]) if member in role }
```

- Default is permissive: a fresh host gives every member `manage` on every agent (line 43). The audit
  diff also uses `"manage"` as the unlisted default for "everyone" (`packages/contracts/src/ipc-server-audit.ts:180`).
- Roles can only raise a member above "everyone", never lower (max-rank loop, lines 44-49).
- `none` makes the agent invisible: `#blockedAgentIds` adds it to the per-member hidden set
  (`src/main/team-api-server.ts:2769-2781`), which (a) 404s any `/v1/agents/:id/...` or `?agentId=`/`?botId=`
  request up front (`team-api-server.ts:1262-1271`) and (b) is scrubbed out of every JSON response
  generically: any object whose `id` or `agentId` is hidden is dropped (`team-api-server.ts:2647-2656`,
  `src/main/team-api/provider-visibility.ts:42-72`).
- Members read only `mine`; admins read the whole `everyone`/`roles` table (`agent-access-store.ts:53-74`).
- Writes: admin only, optimistic `revision`, 409 on stale (`src/main/team-api/route-agent-access.ts:29-41`).
- Store: `openbot-agent-access-v1.json`, keyed by serverId (`agent-access-store.ts:17,107-120`).

Gate helper: `requireAgentLevel(context, agentId, "view"|"chat"|"manage")` 403s below the level
(`src/main/team-api/request-helpers.ts:278-288`); `requireAdmin` / `requireOwner` (`request-helpers.ts:258-264`).

Generic rule for `/v1/agents/:agentId/<action>` (`src/main/team-api/route-agents.ts:228-233, 375-391`):
every non-GET needs `chat` if the action is in `CHAT_ACTIONS` (`messages`, `reactions`, `interrupt`,
`failures/acknowledge`, `queue/edit|cancel|steer|update|reorder`), `view` for `conversation/read|unread`,
and **`manage` for everything else** (PATCH agent, avatar, memories writes, routines writes, duplicate).
GETs need only visibility (level != none). Agent DELETE is admin-only (`route-agents.ts:263-265`).

## 1. Inventory: what a remote member can do on a host, and the gate

Legend: **A** = owner/admin (`requireAdmin`), **O** = owner only, **V/C/M** = agent level view/chat/manage
for plain members (owner/admin always pass), **any** = any authenticated member.
Rows marked (read) I read the handler in full; rows marked (grep) I only located the gate call by grep.

### Chat with an agent (the core)
| Action | Gate | Where |
|---|---|---|
| List agents, status, models, usage | any (hidden/none agents scrubbed) | route-agents.ts:168-213 (read) |
| Read agent conversation / page | V (visible) | route-agent-conversation.ts:45-73 (read) |
| Mark read / unread (own cursor) | V | route-agent-conversation.ts:74-96; route-agents.ts:375 (read) |
| Send message (+attachments, reply-to) | C | route-agent-conversation.ts:97-116 (read) |
| React to message | C | route-agent-conversation.ts:118-134 (read) |
| Interrupt turn, steer/edit/cancel/reorder queued messages | C | route-agent-queue.ts (read) |
| Answer agent question (prompt) | C; privileged UI-block actions need owner/admin | route-agents.ts:313-325; attention-registry.ts:3063-3068 |
| Answer tool approval Allow/Deny | C; some approvals owner-only | route-agents.ts:326-337 (read) |
| Browser takeover done/cancel, browser secret handoff | C | route-agents.ts:338-370 (read) |
| Read pending approval full copy | C | route-pending-approval.ts (read) |
| List pending questions across agents | V | route-pending-prompts.ts (read) |
| Agent waits list / cancel | C | route-agent-waits.ts (read) |
| Side question to an agent | C | route-side-question.ts (read) |
| Delete own message / any message | C own; M any | route-message-delete.ts:91-103 (read) |
| Delete agent chat history | M | route-chat-history.ts:157-164 (read) |
| Context reset / compact | C | route-context-reset.ts, route-context-compact.ts (grep) |
| Pins list/set | C | route-message-pins.ts (grep) |
| Live voice with agent | C | route-live-voice.ts:116 (grep) |
| Message search, file search | any; results scrubbed of hidden agents | route-agents.ts:153-167; route-conversation-files.ts (read) |

### Side threads ("tasks") of an agent
| Action | Gate | Where |
|---|---|---|
| List threads, read thread, read older pages, archived list, branches list | V | route-agent-tasks.ts:55-63; route-agent-threads.ts:220-259 (read) |
| Start thread, send in thread, stop/close/reopen, rename/delete, branch, reply, thread read-marks, thread queue, thread requests, run queued msg as thread | C | route-agent-tasks.ts:57-90; route-agent-threads.ts (read); route-agent-queue-task.ts; route-agent-thread-queue.ts (read) |

### Agent configuration
| Action | Gate | Where |
|---|---|---|
| Create agent (`POST /v1/agents`), generate/save profile | any (!) | route-agents.ts:128-152, 217-226 (read) |
| Edit agent name/title/description/provider/model/effort/avatar | M | route-agents.ts:254-305 (read) |
| Duplicate agent | M | route-agents.ts:259-262 (read) |
| Delete agent | A | route-agents.ts:263-265 (read) |
| Memories create/edit/delete/inclusion | M | route-agent-memories.ts (read) |
| Agent routines writes | M (via `/v1/agents/:id/routines`) | route-agent-routines.ts (grep: no own gate, agentWriteLevel applies) |
| Fast mode / speed: read / change | V / M | route-agent-admin.ts:261-328 (read) |
| Agent colors, chat colors | M | route-agent-colors.ts:30; route-chat-colors.ts:37 (grep) |
| Pending runtime change, usage-limit switch | M (other account needs A) | route-agent-pending-runtime.ts; route-agent-usage-limit.ts (grep); Doc team-api.md:688-714 |
| **Access mode (Full access / Ask first = autoApprove)**, host settings, backup models, task limit, new-chat, plan card, builtin memory, defaults | **A** | route-agent-admin.ts:51-259, 346-389 (read) |
| Agent instructions, session settings, folders (workspace dirs) | A | route-agent-instructions.ts:27; route-agent-session-settings.ts:24; route-agent-folders.ts:29 (grep) |
| Hide agents (host-wide) | A | route-agent-hidden.ts:23 (grep) |
| Agent access table (roles) write | A | route-agent-access.ts:29 (read) |
| Install / publish agent templates, marketplace | A | route-agent-install.ts, route-agent-publish.ts (read) |

### Channels (multi-agent group chats on the host)
| Action | Gate | Where |
|---|---|---|
| List channels, read channel | any; channel hidden only if **all** its agent members are hidden to you | route-channels.ts:93-157, 189-214 (read) |
| Post / save / cancel commands | any visible; adding a new agent member needs C on it | route-channels.ts:168-186, 256-272 (read) |
| Archive / restore / delete channel | A | route-channels.ts:158-173 (read) |
| Channel memories and routines (create/update/delete/test) | any visible (no level check) | route-channels.ts:274-325 (read) |
| Channel picture set/delete | any visible | route-channels.ts:327-357 (read) |
| New conversation / compact; delete history | any visible; delete A | route-chat-history.ts:165-179 (read) |
| Delete own channel message / any | any own; A any | route-message-delete.ts:78-89 (read) |
| Channel folders | A | route-channel-folders.ts:33 (read) |

### People
| Action | Gate | Where |
|---|---|---|
| `GET /v1/me`, presence, logo, logout, change password, leave | any | route-team.ts:71-117 (read) |
| List members, change role (admin/member), disable, remove; invites create/list/revoke; sessions list/revoke | A (non-owner cannot revoke owner's session; nobody changes owner) | route-team.ts:128-208 (read) |
| Direct messages member-to-member (threads, send, read, page) | any, scoped to the pair | route-direct.ts (read) |
| Server audit log read | A | route-server-audit.ts:15 (read) |

### Host machine and shared resources
| Action | Gate | Where |
|---|---|---|
| **Remote desktop (screen + control) session** | policy `access`: default **`members`**, `askOnHost` default **false** | route-remote-screen.ts:34-94 (read); ipc-team-host.ts:493-511; host-service.ts:686-702 |
| Host's shared browser: list tabs, open, navigate, load URL, reload, close, preview, take user control, teach, answer permission/HTTP sign-in, **import cookies/storage into a tab**, view session | any; private tabs and tabs owned by hidden agents excluded for members | route-browser.ts (read) |
| Share own Chrome tabs to the host's agents (extension) | any (own session only) | route-client-browser.ts (read); Doc integrations.md:831-916 |
| Upload attachments; download attachment by ID; shared file by path; workspace file of an agent | any (members get `resolveWorkspaceFile`, admins `resolveLocalWorkspaceFile` with exact path) | route-files.ts:76-170 (read) |
| List an agent's workspace directory | visible (no level check) | route-workspace-directory.ts (read) |
| MCP servers: list (with env/header values), test, **add incl. stdio commands**; change/remove only own rows | any member iff host has `mcp-member-access-v1` AND client sends it; else A | mcp-member-access.ts:50-58; route-mcp.ts (read); Doc integrations.md:86-101 |
| MCP access table read `mine` / write | any / A | route-mcp-access.ts (read) |
| MCP sign-in relay (own rows) | same as MCP | route-sign-in-relay.ts:68-117 (read) |
| Provider sign-ins, accounts, provider settings, env vars, detection | A | route-provider*.ts (grep), route-sign-in-relay.ts:46-67 (read) |
| Vault (secrets, reveal), password managers, connectors (WhatsApp, Discord, 1Password, Bitwarden) | A | route-vault.ts:101 (read section); others (grep) |
| Events/webhooks routines | A | route-events.ts:192 (read) |
| Host settings (timezone, voice, personality, turn limits, restart, update, appearance, signal home, prompt cache, compaction, credits) | A | route-host-*.ts etc. (grep) |
| Host key rotation; "local servers" create/delete | O | route-host-key-rotation.ts (grep); route-local-servers.ts:21 (read) |
| Storage usage | any for usage; rest A | route-storage.ts:87 (grep) |
| Workflows: list/run; admin-only node settings | C on agents in scope; some A | route-workflows.ts:258-269, 408-462 (grep) |

Notable permissive defaults (code): every member manages every agent (agent-access-store.ts:43), every
member "uses" every MCP server (`DEFAULT_MCP_SERVER_ACCESS = { agents: null, everyone: "use" }`,
`packages/contracts/src/ipc-mcp-access.ts:13`), every member may start remote desktop control without a
prompt on the host (`DEFAULT_REMOTE_CONTROL_POLICY = { access: "members", askOnHost: false }`,
`ipc-team-host.ts:501`), and every agent auto-approves tools (`DEFAULT_APPROVAL_AUTOMATION_PREFERENCE =
{ defaultAutoApprove: true }`, `packages/contracts/src/ipc-approvals.ts:78-81`).

## 2. Identity attachment and author attribution

**How identity is attached (code).** Each request carries `Authorization: Bearer <token>`
(`request-helpers.ts:227-230`); the server resolves it with `store.authenticateSession(token)` to
`{ member, sessionId, sessionExpiresAt }` or returns 401 (`team-api-server.ts:1252-1256`), then builds an
immutable `TeamApiRequestContext` with `member`, `sessionId`, `fromChrome`, and a closure
`agentLevel(agentId)` bound to that member (`team-api-server.ts:3189-3212`; type at
`src/main/team-api/request-context.ts:6-21`). Over WebRTC the client never supplies this token: the host's
per-peer object `TeamWebRtcHostPeer` opens a local session from the Signal-authenticated connection
(`membershipId`, role) and injects its own bearer token when it forwards the peer's `http.request` frame
to the local Team API (`src/main/team-webrtc-host-peer.ts:367-387, 802-818`). So the member identity is
bound by the host side to the authenticated peer connection, not asserted by the client.

**What gets stamped on a message.** Routes pass `memberSender(member)` =
`{ id: member.id, name: name || email || username }` (`request-helpers.ts:248-250`) into `sendMessage`,
`startTask`, queue edits, reactions, profile save and agent create (e.g. route-agent-conversation.ts:110,
route-agent-tasks.ts:75). `AgentService.#sendUserMessage` enqueues the delivery as
`sender: { kind: "user" }` plus a separate `senderMember` field (`src/backend/agent-service.ts:7191-7194`),
stored with the mailbox message (`src/backend/mailbox-store.ts:713`). Messages from different members are
never merged into one turn (`mailbox-store.ts:2133-2139`: the batch breaks when `senderMember.id` changes).

**Does the agent see which human wrote it? Direct agent chat: no (code).** The prompt builder
`deliveryPromptInput` (`src/backend/agent/delivery-content.ts:405-633`) only branches on
`delivery.sender.kind`; a human message is rendered as "the user", e.g. replies become
`"The user is replying to message ..."` (line 416-427). `QueueDelivery.sender` has only
`{ kind: "user" }` with no identity (`packages/contracts/src/ipc-queue.ts:37-40`). I found no use of
`senderMember` in prompt text; it is used for UI, pins (`agent/message-pins.ts:184`), delete-own checks
(`agent/message-deletion.ts:108`), side-question transcripts (`agent/side-question.ts:53`), batching, and
`#personTurn`, which checks whether the human is remote (`agent-service.ts:7487-7496`). The only
sender-derived prompt note is `FROM_CHROME_NOTE` when the member sent from their own Chrome
(`delivery-content.ts:403-404, 621`).

**Channels: yes (code).** The channel transcript given to agents is JSON lines including the full
`author` object (`kind`, `id`, `name`) (`src/backend/channel-history.ts:83-99`); the channel command route
passes `{ id: member.id, name: member.name ?? "Team member" }` (`route-channels.ts:181`). Follow-ups from a
human are still labeled "Follow-up from the user" (`channel-history.ts:289-294`), so attribution there is
partial.

Agent-to-agent messages are explicitly attributed: `"Message from OpenBot teammate <name> (<agentId>)."`
(`delivery-content.ts:210, 509`).

## 3. Approvals: who approves when the requester is remote

- **Any member with `chat` on the agent** can answer any pending approval of that agent, whoever started
  the turn: `requireRequestAgent` = not `none` + `chat` (`route-agents.ts:100-104, 326-337`). The approval
  responder only carries `{ owner: member.role === "owner" }`; there is no "requester only" check.
- The same holds for question prompts (`route-agents.ts:313-325`) except that UI-block actions marked
  privileged need owner/admin (`privileged: member.role !== "member"`; `attention-registry.ts:3063-3068`).
- **Owner-only approvals** (`ownerOnly: true`, forces always-ask): vault card fill, NEAR wallet payment,
  Link card fill, phone-app bug report (`src/backend/agent/attention-registry.ts:1135-1136, 1401, 1794,
  1878, 1966`). A non-owner gets 403 `error.backend.approvalOwnerOnly`.
- **Full access vs Ask first is per agent and host-wide, not per member.** It is `ApprovalAutomation`
  (`src/main/approval-automation-store.ts`): `defaultAutoApprove` plus per-agent overrides, default
  `true` (`ipc-approvals.ts:78-95`). `#answersForAgent` auto-accepts unless the workspace is sandboxed
  (`attention-registry.ts:1263-1277, 1411-1418`); `alwaysAsk`/`ownerOnly` approvals still ask. Only owner/admin
  can change it (`route-agent-admin.ts:51-79`, settings carry `access` + `autoApprove`,
  `packages/contracts/src/team-protocol/agent-admin-v1.ts:19-23`). So a member on a Full-access agent
  effectively runs tools on the host with no prompt; on Ask-first, any chat-level member (not only the
  person who asked) can click Allow.
- Media and credits holds: any chat member can accept; `enable` credits and "mute" for everyone are
  owner/admin (Doc team-api.md:623-651; `attention-registry.ts:730`).
- Remote desktop is a separate approval: host-side yes/no dialog only when `askOnHost` is on
  (`host-service.ts:686-702`).

## 4. Isolation between members

- **Agent chat is one shared conversation per agent.** `readConversationFor(agentId, memberId)` reads the
  agent's single `threadId` and adds only that member's read cursor (`src/backend/agent/conversation-reader.ts:44-87`).
  So member X with `view` on an agent reads everything member Y and the agent said there. Per-member state
  is only the read cursor (Doc team-api.md:122-126) and device-local hidden/pinned chats (Doc 252-253).
- **Side threads are per agent, not per member.** `listTasks(agentId)` / `readTask(agentId, taskId)` need only
  `view` (`route-agent-tasks.ts:55-63`), with no owner filter; any `chat` member can stop, close, rename,
  delete or branch a thread someone else started (`route-agent-threads.ts:49-73, 171-218`).
- **Queued messages:** any `chat` member can cancel, reorder, steer, or edit queued messages; the route
  does not check that the message is theirs (`route-agent-queue.ts:193-262`). Message delete does check
  ownership unless `manage` (`route-message-delete.ts:94-101`).
- **Channels** are visible to every member unless every agent in the channel is `none` for them
  (`route-channels.ts:189-191, 206-214`); human membership of channels does not exist.
- **The only member-private conversations are human-to-human DMs** (`route-direct.ts`), keyed by the two
  member IDs (`src/backend/team-chat-store.ts:200-226`), stored on the host.
- **Isolation tools available:** set an agent to `none`/`view` for everyone and grant `chat`/`manage` via a
  custom role. This hides, not partitions: a visible agent's whole history is shared.
- The host's shared browser is shared too: members see all tabs except `isPrivate` tabs and tabs owned by
  hidden agents (`route-browser.ts:57-74`), and can take control or import cookies into a tab.
- The host owner sees everything (local app, DB on disk).

## 5. Audit log (code)

`src/main/server-audit-store.ts` + `packages/contracts/src/ipc-server-audit.ts`.
- File `openbot-server-audit-v1.json`, per serverId, newest first, capped at **500 entries**
  (`server-audit-store.ts:22, 138-167`; `ipc-server-audit.ts:10`).
- Entry: `{ id (uuid), at (ISO), actor: {id,name}|null, change }` (`ipc-server-audit.ts:40-45`).
- Change kinds only (`ipc-server-audit.ts:22-36`): `member-joined` (role), `member-removed`, `member-role`
  (from/to), `role-created`, `role-renamed`, `role-deleted`, `role-member` (added/removed), `agent-level`
  (role|everyone, agent, from, to).
- Sources: agent-access saves, which diff before/after (`agent-access-store.ts:157-181`; wired in
  `application-services.ts:1546`), and member directory changes (`host-service.ts:1715-1726`). Owner rows
  are skipped (`server-audit-store.ts:48, 56, 62`).
- Read: owner/admin only (`route-server-audit.ts:15`). It is a passive read for presence (`client-use.ts:189`).
- **Not audited:** messages, approvals given, tool calls, remote desktop sessions, browser use, MCP
  additions, file downloads, agent edits, vault reveals. It is an access-control change log, not an
  activity log.

## 6. Can a host's agents reach another host's agents?

**Code: no path found.** Agent-to-agent messaging is `openbot.send_message` with `recipientAgentIds`
only (`src/backend/openbot-tools.ts:869-880`), delivered through the host's local mailbox
(`mailbox-store.ts:505-507`). `grep` finds no remote-server references in `src/backend/` (no
`remoteServers` or registry). The Team API has no route that forwards to another host. The cross-device
or cross-network pieces are all "a person's client to one host": the WebRTC transport, the sign-in
callback relay (route-sign-in-relay.ts; Doc integrations.md:169-299), the watch relay (route-watch-relay.ts),
the member's own Chrome tabs (client-browser), and owner-only "local servers" (more servers on the same
machine, route-local-servers.ts). A person who joined servers A and B runs separate connections; their
agents on A cannot message agents on B. The "multiplayer" model is several humans sharing **one host's**
agents, not linking separate people's agents.

Caveat: `route-instance-transfer*.ts` (admin) and `route-agent-import.ts` move agent data between
installs as files, not live messaging. I located these by grep and did not read them in full.

## 7. Docs vs code

- Doc team-api.md matches the code I checked for message-delete (409-421), chat-history (423-433),
  agent-waits (471-477), pending-approval (479-488), thread-queue (663-674), routines-once (543-555), and
  says role checks are centralized in `canAdministerServer` (110-113), which is true.
- Doc integrations.md "Add a Team API capability" (716-749): each feature is a frozen per-capability codec
  in `packages/contracts/src/team-protocol/<name>-v1.ts`, a `route-<name>.ts` that returns `"unmatched"`,
  400 when the capability is off, and `requireAdmin` for admin routes, registered in `TeamApiServer.#handle`.
  The code follows this pattern.
- Doc integrations.md:86-135 matches `mcp-member-access.ts` / `mcp-access-store.ts`: members add stdio
  MCP servers when the host and client both have `mcp-member-access-v1`.

## 8. Coverage

Read in full: `request-context.ts`, `http-error.ts`, `request-helpers.ts`, `mcp-member-access.ts`,
`client-use.ts`, `route-agents.ts`, `route-agent-conversation.ts`, `route-agent-queue.ts`,
`route-agent-queue-task.ts`, `route-agent-access.ts`, `route-server-audit.ts`, `route-team.ts`,
`route-channels.ts`, `route-channel-folders.ts`, `route-agent-threads.ts`, `route-agent-tasks.ts`,
`route-agent-waits.ts`, `route-agent-thread-queue.ts`, `route-agent-active-queues.ts` (lines 20-67),
`route-agent-admin.ts`, `route-pending-approval.ts`, `route-pending-prompts.ts`, `route-phone-feedback.ts`,
`route-direct.ts`, `route-remote-screen.ts`, `route-events.ts`, `route-files.ts`, `route-mcp.ts`,
`route-mcp-access.ts`, `route-browser.ts`, `route-client-browser.ts`, `route-message-delete.ts`,
`route-chat-history.ts`, `route-conversation-files.ts`, `route-side-question.ts`,
`route-workspace-directory.ts`, `route-agent-memories.ts`, `route-sign-in-relay.ts`, `route-watch-relay.ts`,
`route-local-servers.ts`, `route-agent-publish.ts`, `route-agent-install.ts`; `route-vault.ts`
(lines 85-140); all six stores; docs team-api.md in full; integrations.md lines 86-135 and 716-916.
Supporting code read in part: `team-api-server.ts` (dispatch, context, hidden sets), `agent-service.ts`
(sendMessage, personTurn), `attention-registry.ts` (approval responder), `delivery-content.ts`,
`conversation-reader.ts`, `channel-history.ts`, `team-webrtc-host-peer.ts`.

Not read in full (gate tabulated by grep only): the other ~100 `route-*.ts` (providers, provider-accounts,
host-*, whatsapp-*, workflows, routine-*, storage, live-voice, library/sidebar folders, colors, pins,
context-reset/compact, instance-transfer*, agent-import, hermes-import, crm, hosted-sites, skills, etc.),
`dependencies.ts`, `provider-visibility.ts` (read lines 42-82 only), `workflow-visibility.ts`,
`workflow-presence-relay.ts` (lines 1-60), `server-safe-mode-api.ts`.
