# OpenBot: joining someone else's server (src/main/remote-*)

Read-only source study. Nothing was run, installed, or built. All paths are relative to
`openbot-src/openbot/`. "Code does" means behavior traced in the code. "Comment claims" is flagged
separately. These files contain almost no comments, so nearly everything below is code behavior.

## Files read in full (top to bottom)

All under `src/main/`:

| File | Lines |
|---|---|
| remote-server-manager.ts | 2891 (5 chunks) |
| remote-server-client.ts | 606 |
| remote-server-connection-status.ts | 257 |
| remote-server-connections.ts | 247 |
| remote-server-errors.ts | 30 |
| remote-server-event-refresh.ts | 565 |
| remote-server-event-stream.ts | 731 |
| remote-server-host-directory.ts | 241 |
| remote-server-http.ts | 215 |
| remote-server-presence.ts | 65 |
| remote-server-stored-shape.ts | 213 |
| remote-server-store.ts | 405 |
| remote-server-summaries.ts | 75 |
| remote-server-urls.ts | 116 |
| remote-session-cache.ts | 174 |
| remote-team-decoding.ts | 113 |
| remote-team-directory.ts | 251 |
| remote-agent-decoding.ts | 316 |
| remote-conversation-decoding.ts | 173 |
| remote-host-decoding.ts | 25 |

### Imports followed briefly (partial reads)

- `src/main/application-services.ts`: lines 572-573 (file names), 1669-1673 (session cache construction), 3868-3935 (manager construction and the WebRTC transport wiring to `centralAuth`).
- `src/main/account-service-access.ts`: lines 1-49 (`safeStorageCipher`, the central auth URL `https://api.openbot.run`).
- `src/main/secret-storage-cipher.ts`: lines 40-164 (the cipher).
- `src/main/team-store.ts`: lines 1558-1560 (`fingerprint`).
- `src/main/team-webrtc-client-transport.ts`: lines 930-1130, 1240-1299, 1355-1404 (session, ticket, and the Ed25519 auth handshake).
- `src/main/browser-egress-client.ts`: lines 1-60.
- `src/main/index.ts`: lines 1686-1698 (who consumes the manager's events).
- `src/main/watch-relay-commands.ts`: lines 220-240. `src/main/dynamic-island-actions.ts`: lines 40-60.
- `packages/contracts/src/team-api-routes.ts`: grep for route constants only.
- grep: every file that references `remoteServers` or `RemoteServerManager`. Also checked that `src/backend` has no reference to the remote-server code.

### Skipped

- `remote-server-urls.test.ts` (a test).
- `team-webrtc-client-transport.ts` outside the ranges above (1612 lines).
- `team-webrtc-bridge`, `central-auth-manager`, `remote-viewer-proxy`, `remote-attachment-cache`, `remote-picture-cache`, `remote-service-effects`, `@openbot/team-client/*`, and the host-side Team API server. Their details are inferred only from the call sites above.

---

## Q1. How a desktop joins someone else's server, and what is stored

### Two transports

A server record carries `transport?: "webrtc-v2"` (remote-server-stored-shape.ts:29).

- **WebRTC v2 (the production path).** The server is reached by host id through an account-service signaling relay. Its stored `apiUrl` is just the placeholder `webrtc://${hostId}` (remote-server-host-directory.ts:152).
- **Legacy direct HTTP.** The server is reached at a real `apiUrl` with a bearer session token. This path is used for local or development servers, for self-hosted servers, or when no WebRTC transport exists.

### Join flow, `RemoteServerManager.join` (remote-server-manager.ts:1003-1112)

1. Parse the invite URL with `parseInviteUrl` (manager:556-563). The payload holds `apiUrl`, `serverId`, `fingerprint`, and `token` (see `createInviteUrl` in remote-team-directory.ts:203-214). A self-hosted build refuses invites for another origin (manager:559-561).

2. **WebRTC branch** (manager:1011-1066), taken when a transport exists and `apiUrl` is not localhost:
   - `transport.previewInvite(invite.token)` calls `centralAuth.previewRemoteInvite`, which goes to the account service (application-services.ts:3925).
   - The code checks `preview.hostId === invite.serverId` (manager:1013).
   - It checks that `fingerprint(preview.devicePublicKey) === invite.fingerprint`. If the fingerprints differ, it follows signed key handovers instead (manager:1015-1031). The fingerprint is the SHA-256 of the PEM text, encoded base64url (team-store.ts:1558-1560).
   - `transport.acceptInvite(invite.token)` calls `centralAuth.acceptRemoteInvite` (manager:1032-1034). An `owner_membership_protected` error is treated as success, so an owner can open their own invite (manager:2837-2840).
   - If the accepted host runs on this computer, the code only selects it (manager:1035-1046).
   - Otherwise it un-hides the host, calls `#syncWebRtcHosts()` to re-list hosts from the account service, and requires that the listed fingerprint still match (manager:1051-1058). Then it selects the host and calls `transport.connect(hostId)` (manager:1060-1063).
   - The server record is never written directly. It comes from the account-service host list (`reconcileWebRtcHosts`, see below).

3. **HTTP branch** (manager:1067-1109):
   - `client.verifyIdentity(apiUrl, serverId, fingerprint)` (remote-server-client.ts:440-486). This negotiates the protocol, then sends `GET /v1/identity?challenge=<24 random bytes>`. It checks four things: `serverId`, `challenge`, `fingerprint`, and `fingerprint(publicKey)`. It also checks `crypto.verify(null, challenge, publicKey, signature)`.
   - `centralAccount.createTeamAuthTicket(serverId)` gets an account-service ticket (manager:1068-1070).
   - `POST /v1/join/account` with body `{ inviteToken, accountTicket }` (manager:1071-1076) returns `{ member: { role }, sessionToken }` (remote-team-decoding.ts:14-22).
   - The code stores the record with the sealed `sessionToken` (manager:1089-1100) and starts the event stream (manager:1108).

4. **Other entry points**:
   - `login` re-authenticates with `POST /v1/auth/account { accountTicket }` (manager:1294-1347).
   - `attachLocalServer` uses `POST /v1/auth/login { username, password }` for a server that runs on this computer (manager:1170-1215).
   - `connectDevelopmentServer` stores a session token it is given (manager:1114-1149).

### What is stored per server

`StoredRemoteServer` (remote-server-stored-shape.ts:17-37) holds:

- `id`, `name`, `apiUrl`
- `fingerprint`, and optionally `publicKey` (the host's pinned identity key)
- `username` (the account email, lowercased)
- `encryptedToken`
- `remoteDesktopAvailable`, `logoVersion`, `appearance`, `role` (owner, admin, or member)
- `transport`
- plan and hosting metadata: `memberLimit`, `hosted`, `storageGb`, `paymentRequired`, `customPlan`, and others

The file-level state (stored-shape:49-59) holds `version: 3`, `activeServerId`, `servers[]`, `hiddenHostIds`, `mutedServerIds`, `serverNotifications`, `browserEgressServerIds`, and `unreadableServers` (records it cannot parse, kept as-is).

- The file is `userData/openbot-remote-servers-v1.json` (application-services.ts:572). It is plain JSON, written atomically (remote-server-store.ts:390-395).
- Only `encryptedToken` is encrypted. Everything else is plaintext, including the fingerprints, public keys, email, and role.
- WebRTC servers get `encryptedToken: ""` (host-directory:156). They have no long-lived token on disk. Their credential is a per-host session that is cached separately (see below).
- `detachLocalServer(forget=false)` seals `""` and sets `apiUrl` to `http://127.0.0.1:0` (manager:1230-1231).

### Encryption at rest

- `sealToken` returns `cipher.encrypt(token)` as base64. `token()` decrypts it (store:143-149).
- The cipher is `safeStorageCipher(...)` (application-services.ts:3870), which builds a `createSecretStorage` keyed by `userData/secret-storage.key` (account-service-access.ts:17-28). In secret-storage-cipher.ts:
  - On macOS and Windows, the default is Electron `safeStorage.encryptString` (the OS keychain or DPAPI) (secret-storage-cipher.ts:130-132).
  - On Linux, it always uses AES-256-GCM with a random 32-byte key file (mode 0600) when that file exists (secret-storage-cipher.ts:100-107, 134-137).
  - On Linux and macOS, it creates that key file when `safeStorage` is unavailable (secret-storage-cipher.ts:61-98). If `safeStorage` cannot decrypt on Linux, it also creates the key file (secret-storage-cipher.ts:152-159).
  - On a Linux machine with a key file, the key sits next to the ciphertext. That protects against casual copying, not against an attacker who can read the user's files.
- `RemoteSessionCache` (remote-session-cache.ts) is `userData/openbot-remote-sessions-v1.bin` (application-services.ts:573, 1669-1673).
  - It holds `{ version, principalId, signalUrl, sessions: { [hostId]: { sessionId, expiresAt } } }`. The whole file is encrypted with the same `safeStorageCipher` (session-cache:125-143) and decrypted on read (session-cache:100-113).
  - It is only written when `canPersist()` is true and the `keepBetweenRuns` preference is enabled (session-cache:39-46, application-services.ts:1671). It is cleared on sign-out (application-services.ts:1674-1678).
  - It caches WebRTC session ids so the app can reconnect without starting a new session.
- No private keys are stored by these files.
  - The client's WebRTC key pair is a fresh Ed25519 pair, generated on every connect and kept only in memory (team-webrtc-client-transport.ts:946-952).
  - Host public keys are pinned in memory with `transport.pinHostKey`, from the stored `publicKey` (manager:608-611, 2731).

## Q2. Can one install join several servers at once? Yes.

- `servers: StoredRemoteServer[]` holds any number of servers (stored-shape:52). `adopt` appends and dedupes by id (store:174-182). `reorder` and `replaceServers` work over the whole list (store:235-266).
- All per-server state is kept in maps keyed by `serverId`:
  - `RemoteServerConnections.#statuses` (connections:36)
  - `RemoteEventStream.#controllers`, `#sockets`, `#reconnectTimers`, and more (event-stream:126-136)
  - `RemoteEventRefresh` caches for rosters, conversations, queues, browser state, colors, and usage limits (event-refresh:60-79)
  - the manager's `#syncedLevels`, `#hostedStartAt`, and more (manager:312, 319-336)
- `RemoteEventStream.start()` calls `ensure()` for every server (event-stream:163-166). Each HTTP server gets its own WebSocket, and each WebRTC server gets its own `transport.connect` (event-stream:373-414).
- `refreshRuntimeSnapshots` loops over all servers (event-stream:184-213).
- One server is "active" (`activeServerId`, store:92-94, 151-156). The active server, or a server shown in a chat window, receives `includeConversations: true` in its `agent-event-scope` frame. Other servers get roster and status events only (event-stream:668-690, manager:830-832).
- `ComputerActivityReport` sends idle-seconds reports to every online server that supports the capability (manager:380-401).
- Every event the manager emits carries `serverId` (manager:214-226).
- `LOCAL_SERVER_ID` (this computer's own host) is always listed first, as `kind: "local"` (summaries:16-34).

## Q3. Does a joined desktop run agents locally, or only act on the host's agents? It only acts remotely.

- Every agent operation on a remote server is a Team API request to the host. Examples:
  - `TEAM_API_ROUTES.agents.all` (event-refresh:195, 227)
  - `agent.conversationPage` (event-refresh:528-535)
  - `agent.queue` (event-refresh:205, 510)
  - `agent.duplicate` (manager:1663-1667)
  - `agent.conversation` and `conversationRead` (manager:1683-1744)
  - `messages.search` (manager:1706-1734)
  - avatars, attachments, workspace files, and shared files (manager:2070-2637)
  - agent import as zip upload (manager:2112-2179)
  - instance transfer (manager:2181-2270)
  - remote desktop sessions (manager:1953-2068)
  - browser view streams (manager:2013-2061)
- Sending a message to a remote agent is also a request: `POST /v1/agents/:id/messages` through `remoteServers.request`, from ipc/agent-handlers.ts:1222, dynamic-island-actions.ts:56, and watch-relay-commands.ts:237. The agent runs on the host.
- `headlessHost` turns off event connections, which is the server-mode case (manager:639, 874, 882).

### Request routing (remote-server-client.ts:127-164, 166-249)

- **WebRTC servers.** `transport.request(hostId, path, init)` or `transport.requestResponse(...)` sends the same Team API path as an RPC over the WebRTC data channel (client:136-143, 177-207). There is no HTTP and no bearer token. Authorization comes from the authenticated WebRTC session (Q5). A JSON body is converted with `decodeTeamProtocolV2Json` (http:152-168). The WebRTC compatibility check is pinned to protocol 2 (connection-status:248-256).
- **HTTP servers.** `fetch(new URL(path, apiUrl))` with these headers (http:73-80, client:213-224):
  - `Authorization: Bearer <decrypted sessionToken>`
  - the protocol-version header (`TEAM_PROTOCOL_VERSION_HEADER`)
  - the app-version header (`TEAM_APP_VERSION_HEADER`)
  - capability headers
  - `Accept: application/json`

  The timeout is 15 s (http:19).
- Requests are not signed per request. The only signatures in these files are the host's identity proof (HTTP path) and the WebRTC auth handshake (Q5).
- Compatibility is negotiated with `GET /v1/compatibility` (client:371-438). An HTTP client offers protocols 1 to 10 (connection-status:10).
- The account-service control plane is used for members and invites on WebRTC servers (remote-team-directory.ts:247-250). Operations there call `centralAuth.listRemoteMembers`, `createRemoteInvite`, and so on (application-services.ts:3922-3934). HTTP servers use `/v1/team/members` and `/v1/team/invites` on the host itself (team-directory:90-92, 120-125, 161-163, 240-242).

## Q4. Do agents on one host call agents on another host? Absent from these files.

What I checked:

1. The manager has no code that takes agent output from one server and sends it to another. Its only outputs are:
   - `emit(...)` to in-process listeners (manager:214-226)
   - requests to the same `serverId` that the call targets
   - `#background` requests, which are all scoped to a single `serverId`: focus ping, computer activity, notification levels, roster, capabilities (manager:338-340, 394-398, 490-519)
2. `RemoteEventRefresh.forward` turns a host's event into local `emit` plus re-fetches from **the same** server (event-refresh:92-156). Nothing in it writes to another server.
3. The consumer of `remoteServers.on("agent", ...)` is `forwardAgentEvent` in index.ts:1690-1692, which pushes to renderer windows. The other listeners at index.ts:1688-1698 also forward to the UI.
4. `grep remoteServers|RemoteServerManager` finds only `src/main` IPC handlers, renderer forwarders, the main window, `message-link-relay`, `browser-view-client`, `local-servers`, and `development-remote-bootstrap`. `src/backend`, where local agents run, never references the remote-server manager. So a local agent has no tool path to a remote server's agents through this code.
5. Every send to a remote agent's `/messages` endpoint found by grep comes from a human UI surface: the renderer IPC, Dynamic Island, or Apple Watch relay.

Agent-to-agent messaging inside one host is out of scope for these files. See the D-agent-comms notes for that.

**Related reverse path: browser egress.** This is not agent-to-agent. If the user turns on "share this computer's browser" for a server:
- The client registers through `POST BROWSER_PROXIES_ROUTES.computer` and records the server in `browserEgressServerIds` (manager:850-869, store:330-351).
- `BrowserEgressClient` then opens an egress channel over the WebRTC bridge (browser-egress-client.ts:30-42). It makes TCP connections from the joining desktop on behalf of the host's browser, and the top of the file imports `isPublicAddress`.

So the host's agents can use the joiner's network. Nothing in these files runs a host's agent on the joiner. `client-browser-command` events are explicitly ignored here (manager:2762, event-stream:577).

## Q5. What credential the joining client presents, and how it gets it

### HTTP transport

1. The client first verifies the **host**, not the reverse. It sends a challenge to `/v1/identity`. The host returns `{ serverId, publicKey, serverName, fingerprint, challenge, signature, logoVersion }` (remote-team-decoding.ts:24-43). The client checks the signature with `verify(null, ...)` and compares `fingerprint(publicKey)` to the fingerprint pinned in the invite (client:454-479). A `null` algorithm matches Ed25519 keys.
2. The client then gets an **account-service ticket** from `centralAuth.createTeamAuthTicket(serverId)` (manager:230-231, 1068; application-services.ts:3872). The account service is `https://api.openbot.run` in packaged builds (account-service-access.ts:45-49).
3. It sends `inviteToken` and `accountTicket` to `POST /v1/join/account`, or only `accountTicket` to `/v1/auth/account` to log in again. The host returns a **session token** (`sessionToken`) (manager:1071-1076, 1305-1308).
4. The session token is the bearer credential on every later HTTP request (http:76, client:148, 215). It is also used in the WebSocket subprotocol `openbot-token.<token>` for `/v1/events` (event-stream:478-480) and for HTTP browser-view streams (manager:2049).
5. When the event socket fails before it opens, the client probes `GET /v1/me`. A 401 or 403 there means it must sign in again (event-stream:637-649, 692-711).

The host's checks are not visible here. Inferred from what the client sends:
- The host must validate the ticket against the account service and redeem the invite token. The host-side code was not read.
- Error codes the client handles show what the host enforces: `authentication_required` on 401 (connection-status:124-137), `session_revoked` (connection-status:159-185, manager:552), and `owner_membership_protected` (manager:2839).

### WebRTC transport (team-webrtc-client-transport.ts, partial read)

1. `centralAuth.startRemoteSession(hostId)` returns `{ sessionId, expiresAt }` (transport:957-963; application-services.ts:3920). This session id is the value cached in `RemoteSessionCache`.
2. The client generates a fresh **Ed25519** key pair (transport:946-952). This is not P-256.
3. `centralAuth.issueRemoteSessionTicket(sessionId, clientPublicKey)` returns `{ ticket, signalUrl }` (transport:965-970, 1012; application-services.ts:3921). The account service binds the ticket to the client key.
4. The bridge connects to `signalUrl` (a wss URL, validated at session-cache:166-174) with `token: ticket` (transport:1116-1123).
5. After the data channel opens, the client sends `auth-init { ticket, clientPublicKey, clientNonce, signature }`. The signature is Ed25519 over a transcript of `hostId`, `sessionId`, `ticket`, `clientPublicKey`, `clientNonce`, and both DTLS fingerprints (transport:1259-1276). This binds the handshake to the WebRTC channel.
6. The host answers `auth-ready { clientNonce, hostNonce, signature }`. The client checks that signature against the **pinned host public key**, which comes from the account-service host list and was checked against the invite fingerprint at join time (transport:1372-1393). Then it sends `auth-complete` (transport:1396-1404).

So on WebRTC, the joiner presents an account-service-issued ticket bound to an Ed25519 key, plus proof that it holds that key. Membership itself lives in the account service: invites are accepted and members listed through `centralAuth` (application-services.ts:3922-3934). `RemoteTeamDirectory` sends member and invite management for WebRTC servers to that control plane, not to the host (team-directory:19-38, 247-250). The invite URL's `apiUrl` is `transport.controlPlaneUrl` (team-directory:204-205), set to `centralAuth.resolveApiUrl("/")` (application-services.ts:3935).

### Host key rotation

- `followHostKeyHandovers` and `verifyHostKeySignature` accept a new host key only through a chain of handovers signed by the trusted key (host-directory:86-114).
- `reconcileWebRtcHosts` keeps a stored public key over an advertised one. It drops the key when the pinned fingerprint and the advertised fingerprint differ, unless a handover was followed (host-directory:144-153, 228-233).
- `keyChangedAt` is surfaced to the UI (manager:2733-2738).

## Q6. What crosses the network

### Event stream (HTTP transport)

- It is a **WebSocket**, not SSE: `wss://<apiUrl>/v1/events` (event-stream:476-481). Subprotocols are `TEAM_PROTOCOL_V1_WEBSOCKET` plus `openbot-token.<sessionToken>`. Older peers use `openbot-events-v2`, `openbot-events`, and the token.
- Only text JSON frames are accepted. Binary frames are rejected (event-stream:524-531), and frames over 1 MiB are rejected (event-stream:50, 533-540).
- Frames are decoded with `optionalTeamEvent` or `decodeTeamProtocolV8BaseCurrentEvent` (event-stream:543-557).
- Event types from host to client:
  - team events: `team-identity`, `team-presence`, `team-direct-message`, `team-direct-typing`, `host-restart`, `host-appearance`, `machine-alert`, `live-voice`, `client-browser-command` (ignored) (event-stream:561-577)
  - agent events, which are everything else (event-stream:578-590). Those include `runtime-snapshot`, `agents-changed`, `conversation-invalidated`, `queue-invalidated`, `agent-colors-changed`, `chat-colors-changed`, the `browser-*` events, `status`, `turn-started`, `turn-completed`, and `conversation` (event-refresh:99-155).
- Frames from client to host:
  - `agent-event-scope { includeConversations, capabilities }` (event-stream:677-689)
  - `runtime-snapshot-request` (event-stream:204)
  - `team-typing` and `team-direct-typing` (manager:1846, 1912-1916)
  - the workflow spot control (event-stream:221-225)
- The event stream mostly carries **invalidation signals**. Content is then pulled over the API. For example, `conversation-invalidated` triggers a fetch of the latest 50-message conversation page (event-refresh:38, 427-468), and `queue-invalidated` triggers a fetch of the queue. When the stream first opens, the client pulls the full roster, plus one message and the queue for each agent (event-refresh:189-216).

### Event stream (WebRTC transport)

- Events arrive through `transport.on("event")` and are routed by the same type switch (manager:536, 2751-2764).
- Typing and workflow-spot updates go out as transport RPCs (manager:1837-1845, 1926-1932).
- On connect, the client sends an `event-ack { throughSequence }` frame (transport:1289-1297), so the host can resume the event sequence.

### Payloads (decoders)

- **Agent summary** (remote-agent-decoding.ts:119-143): `id`, `provider`, `name`, `title`, `description`, `notifications`, `model`, `reasoningEffort`, `threadId`, **`workspacePath`** (the host's filesystem path), `preview` (the latest message snippet), `updatedAt`, avatar fields, `marketplaceSource`, and `pendingRuntime`. The decoder strips `backupModels` and `usageLimitReturn` (remote-agent-decoding.ts:120).
- **Conversation page** (remote-conversation-decoding.ts:120-132): `agentId`, `threadId`, `activeTurnId`, `revision`, full `messages[]` (checked with `isConversationMessage`), `references`, `pageInfo`, and `readState`.
- **Direct messages between members**: `{ id, threadId, senderMemberId, recipientMemberId, text, createdAt, sequence }` (remote-conversation-decoding.ts:25-36).
- **Presence snapshot**: `members` with an `online` flag and `typingAgentId` (remote-server-presence.ts:49-58, remote-team-decoding.ts:45-51).
- **File bytes**: attachments, workspace files (`/v1/workspace-files?botId&path`), shared files, avatars, the logo, agent-import zips, instance-transfer chunks with sha256 (manager:2070-2637), plus remote-desktop and browser-view streams.
- **Also**: the client's idle seconds (manager:394-398), a focus-use ping (manager:338-340), notification levels, and colors.
- Host preview and avatar URLs are rewritten to local custom schemes (`openbot-remote-attachment://`, `openbot-remote-avatar://`, `openbot-remote-server-logo://`) so the renderer never sees the host URL (remote-server-urls.ts:22-116).

### Encryption in transit

- HTTP uses TLS when the `apiUrl` is https. The ws or wss choice follows the scheme (event-stream:477).
- WebRTC traffic is DTLS between peers, and the auth transcript binds both DTLS fingerprints (Q5).
- No encryption at the application layer was seen.

## Code vs. comments

These 20 files have almost no prose comments, so no claims in comments needed checking. Everything above is code behavior.

User-facing strings referenced by key, such as `error.remote.inviteIdentityMismatch` and `error.remote.signInAgain`, match the checks that raise them.

One wording note: `AGENTS.md` in `main/ipc` was not read.
