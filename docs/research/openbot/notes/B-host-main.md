# B. OpenBot main process: hosting and joining servers

Read-only study of `openbot/src/main/`. Nothing was run, installed, or built. Paths are relative to `openbot/src/main/` unless they start with `packages/`, `apps/`, or `remote/`. "M:" means `central-auth-manager.ts`. The claims come from code unless marked DOC (AGENTS.md, design notes) or "comment".

This file merges four reading passes:
- **Core**: team-api-server, team-store, host-service, signal-ingress, host-signal-home, server-mode, AGENTS.md, design notes. I read these myself, and the detailed notes are in `B-host-main-core.md`.
- **Remote**: the `remote-server-*`, `remote-team-*`, `remote-session-cache`, and `remote-*-decoding` files. Notes: `B-host-main-part-remote.md`.
- **Auth**: the `central-auth-*`, `account-service-*`, `stored-p256-key`, `secret-storage-cipher`, `mobile-connect-host`, and `watch-relay-*` files. Notes: `B-host-main-part-auth.md`.
- **WebRTC**: the `team-webrtc-*` files. Notes: `B-host-main-part-webrtc.md`.

Each part file has the full citations and its own read and skip lists.

---

## Architecture in one paragraph

Every desktop runs its own agents. A desktop becomes a **host** by publishing its local Team API, an HTTP and WebSocket server on 127.0.0.1. It publishes through OpenBot's cloud:
- the **account service** (`https://api.openbot.run`) handles identity, membership, invites, and host registry;
- the **Signal** WebSocket relay handles WebRTC signaling and the integration ingress;
- **coturn** provides TURN.

A joining desktop opens a WebRTC data channel to the host and tunnels Team API HTTP requests and events through it. The joiner runs nothing on the host's behalf except optional browser egress. It is a remote control for the host's agents. Identity and roles are asserted by the account service as ES256 JWTs. The channel is DTLS, bound to both peers by an Ed25519 handshake. The host sees and stores everything in plaintext.

---

## Q1. What "hosting" means

**When an install is a host.** `TeamStore` has an active `StoredTeam` for the signed-in account (`team-store.ts:378`, `:286-291`). The file can hold several `hosts[]`, one per owner account, but only one is active at a time (`#state`, `:194`). `HostService.register()` creates a server named "OpenBot" automatically when none exists (`host-service.ts:1012-1016`). Publishing requires that the signed-in account owns the server (`assertOwnerAccount`, `host-service.ts:1085`; `team-store.ts:403-414`).

**The local Team API server.**
- Node `http.createServer` plus a `ws` WebSocketServer (`noServer`) (`team-api-server.ts:633-644`, `:720`).
- It is bound to **127.0.0.1 only**, on an OS-assigned port: `team-api-server.ts:780` → `listen-loopback.ts:19`, `server.listen(port, "127.0.0.1", …)`.
- Routing is a long ordered dispatcher (`team-api-server.ts:1140-1594`), delegated to `team-api/route-*.ts`. Gates run in this order (DOC AGENTS.md:125-131, code `:1160-1256`):
  1. compatibility
  2. remote-screen
  3. protocol headers (426 or 400)
  4. unauthenticated join and login routes
  5. `Authorization: Bearer`
- Events use a WebSocket at `/v1/events`, with the token in the subprotocol `openbot-token.<token>` (`:737-771`). Events are filtered per member and per capability (`:1866-2115`).
- Remote peers never reach the port directly. The **WebRTC host peer** forwards to it with `fetch("http://127.0.0.1:<port>/v1/…")` and refuses any URL whose origin changes or whose path lacks the `/v1/` prefix (`team-webrtc-host-peer.ts:696-745`, `:747-903`).
  - Headers on the forwarded request: `Authorization: Bearer <per-session local token>`, `X-OpenBot-WebRTC-Session`, and the protocol, app-version, and capability headers.
  - The legacy direct-HTTP transport exists for development and self-hosted servers (part-remote Q1).

**Registration with the account service** (`host-service.ts:1069-1148`, `#startRuntimeOperation`):
1. `api.start()` returns the loopback port.
2. `registerRemoteHost({hostId, name, ownerMembershipId, devicePublicKey})` sends `POST /v2/remote/hosts/register` (`host-service.ts:991-1010`; M:603-639). `devicePublicKey` is the host's **Ed25519** SPKI PEM key. The response is a per-host `machineToken`, stored encrypted in `openbot-central-auth-v1.bin`.
3. In parallel, two calls run:
   - `listRemoteMembers` → `syncRemoteDirectory`. The account service is the member directory (`host-service.ts:1105-1112`; `team-store.ts:839-917`).
   - `issueRemoteHostTicket` → `POST /v2/remote/hosts/:id/ticket {machineToken, signalHome,…}`, which returns `{ticket, signalUrl}` (M:769-792).
4. `TeamWebRtcHostGateway.start({hostId, signalUrl, ticket, localApiPort})` (`host-service.ts:1122-1128`). The hidden renderer opens the Signal WebSocket with `hello {peer:"host", token: ticket, multiplex}`. One host socket multiplexes many clients by `connectionId` (part-webrtc Q2).

**Registration with Signal.** The host never registers with Signal directly. It presents the account-service ticket. The Signal service is chosen from "signal homes" the account service lists (`POST /v2/remote/hosts/:id/signal-homes`, M:794-807). The host measures `/health/ready` latency and keeps a preferred home in `openbot-signal-home-v1.json` (`host-signal-home.ts:46-71`, `:130-136`).

**Signal ingress.** `signal-ingress.ts` opens a second outbound socket (`peer:"ingress"`) with the same kind of host ticket (`:605-629`). It carries Slack, Discord, Telegram, webhook, GitHub, and watch-relay deliveries from public endpoints into the host (`:710-865`). Signal can hold deliveries for an offline host, sealed to the host's `queueKey.publicKey`. The host opens them with `openQueuedDelivery(privateKey, hostId, sealed)` (`:673-694`).

**Headless "server mode."** It requires a packaged Linux build and `OPENBOT_SERVER=1` (`server-mode.ts:175-188`). Control is an HTTP server on the unix socket `$XDG_RUNTIME_DIR/control.sock`, with checks that the directory is private and the socket is chmod 0600 (`:240-277`). Routes cover status, email-code login, server-claim login, rename, a Mobile Connect QR code, and logout (`:385-409`). `publish()` configures the server and calls `host.start` (`:290-320`).

---

## Q2. Joining someone else's server

**Join flow, WebRTC (production)** (`remote-server-manager.ts:1003-1066`):
1. Parse the invite URL `{apiUrl, serverId, fingerprint, token}`. The `apiUrl` is the account-service control plane (`host-service.ts:1848-1857`).
2. `previewRemoteInvite` at the account service. Check `preview.hostId === invite.serverId`, and that `fingerprint(preview.devicePublicKey) === invite.fingerprint`, or follow signed key handovers.
3. `acceptRemoteInvite` at the account service.
4. Re-list hosts, re-check the fingerprint, then `transport.connect(hostId)`.

The server record is derived from the account-service host list. There is no per-server long-lived token: `encryptedToken: ""` (`remote-server-host-directory.ts:152-156`).

**Join flow, legacy HTTP** (`manager:1067-1109`):
1. `GET /v1/identity?challenge=<24 random bytes>`. The host signs the challenge with Ed25519 (`team-store.ts:416-421`; `team-api-server.ts:1172-1179`). The client verifies it against the pinned fingerprint (`remote-server-client.ts:440-486`).
2. `createTeamAuthTicket` at the account service.
3. `POST /v1/join/account {inviteToken, accountTicket}`. The host calls `redeemCentralTicket` at the account service (`team-api-server.ts:1200-1221`) and mints a 30-day `sessionToken` (`team-store.ts:1441-1461`).

**Several servers at once: yes.**
- `servers: StoredRemoteServer[]` (`remote-server-stored-shape.ts:52`).
- Per-server maps hold status, sockets, reconnect timers, and caches (`remote-server-connections.ts:36`; `remote-server-event-stream.ts:126-136`; `remote-server-event-refresh.ts:60-79`).
- `RemoteEventStream.start()` calls `ensure()` for every server, with one event connection each (`event-stream:163-166`, `:373-414`).
- One `activeServerId` receives conversation-level events (`includeConversations: true`). The others receive roster and status only (`event-stream:668-690`).
- The desktop's own host is always listed first as `LOCAL_SERVER_ID` (`remote-server-summaries.ts:16-34`).
- Separately, `LocalServers` can run extra *local* server child processes on one machine (application-services.ts, `new LocalServers`, `agentRoot: ~/OpenBot/Servers`).

**Local agents or proxy? Proxy only.** Every agent action on a joined server is a Team API call to the host: list agents, conversation pages, queue, duplicate, search, files, import, instance transfer, remote desktop, and browser view (part-remote Q3, manager:1663-2637).
- Sending a message is `POST /v1/agents/:id/messages` through `remoteServers.request`. The callers are the renderer IPC (`ipc/agent-handlers.ts:1222`), Dynamic Island, and the Apple Watch relay.
- On WebRTC, requests are `{type:"request", operation:"http.request", payload:{method, path, body, capabilities}}` frames on the `rpc` data channel, with no bearer token (`team-webrtc-client-transport.ts:489-644`).
- Agents always execute on the host.

---

## Q3. Do agents on host A call agents on host B? Absent

Evidence:
- `src/backend`, where agents run, never references `RemoteServerManager` or `remote-server*`. I grepped for it. The only backend hit for `/v1/agents` is `backend/automation-command.ts:11-43`, which builds a curl or PowerShell command against the **local** Team API, using a local token file in `userData/automation`.
- `RemoteServerManager` emits only to in-process listeners. Those are renderer forwarders (`index.ts:1690-1692`). Its requests are always scoped to the `serverId` the caller named (part-remote Q4).
- The host peer forwards only to its own loopback API. It drops an `incoming` event whose peerId is its own hostId (`team-webrtc-host-gateway.ts:354`; `team-webrtc-host-peer.ts:704-709`).

Agent-to-agent messaging exists only *within* one host (channels, subagents; see D-agent-comms notes). Other cross-machine paths exist, but none of them is agent-to-agent:
- **Browser egress**: the host's agent browser exits through a connected member's network over the `openbot.browser-egress.v1` channel (`browser-egress-host.ts:55-66`).
- **`client-browser-v1`**: host agents send `client-browser-command` events (open, navigate, click, type, screenshot) to a member's browser session (`team-api-server.ts:1011-1026`; `packages/contracts/src/team-protocol/client-browser-v1.ts`). The desktop client ignores these events (`remote-server-manager.ts:2762`). The `fromChrome` request flag (`team-api-server.ts:3207`) suggests a browser-extension client handles them.
- **Sign-in relays and provider-account relays**: OAuth pages are opened on the member's computer for the host (DOC design notes:241-253).
- **Direct messages**: person-to-person on one host (`team-api-server.ts:1076-1118`, `:2522-2560`).

---

## Q4. Data at rest

| What | File (userData) | Protection | Cite |
|---|---|---|---|
| Host Ed25519 **private key** (PEM), members, invite-token hashes, session-token hashes, scrypt password hashes, pending key rotation | `openbot-team-server-v2.json` | **Plaintext JSON**, file mode 0600 and directory 0700 only | `team-store.ts:74`, `:110-128`, `:1531-1555`; `backend/atomic-json-file.ts:55,88` |
| Account `sessionToken`, per-host `machineToken`s, Mobile Connect link, user | `openbot-central-auth-v1.bin` | `secret-storage-cipher` | M:2584-2596 |
| Joined servers list (id, name, apiUrl, fingerprint, publicKey, email, role, plan) | `openbot-remote-servers-v1.json` | Plaintext except `encryptedToken` | `remote-server-store.ts:143-149`, `:390-395` |
| WebRTC session ids per host | `openbot-remote-sessions-v1.bin` | Whole file encrypted; written only if the keep-between-runs preference is on | `remote-session-cache.ts:100-143` |
| Watch-relay P-256 **ECDH** key | `openbot-watch-relay-key-v1.json` | Public key plaintext, private key encrypted | `stored-p256-key.ts:39-60` |
| Fallback AES key | `secret-storage.key` | 0600 only | `secret-storage-cipher.ts:61-98` |

**The cipher** (`secret-storage-cipher.ts`):
- The primary path is Electron `safeStorage`: macOS Keychain, Windows DPAPI, or Linux libsecret/kwallet (`:130-133`).
- On Linux and macOS there is a fallback: a 32-byte AES-256-GCM key in `secret-storage.key`, next to the ciphertext. The ciphertext format is `"obk1"||iv||tag||ct` (`:20-23`, `:134-137`).
- On Linux, once the key file exists it wins for all new writes (`:100-107`). It is created when `safeStorage` is unavailable or a decrypt fails (`:151-159`).
- Windows has no fallback; encryption fails.
- The code does not reject Electron's `basic_text` backend on Linux.

**Conclusion:** the most sensitive host secret, the Ed25519 identity key, is **not** encrypted at rest. Session and bearer tokens are, with the caveats above. Conversation data lives in the backend stores, outside this area.

---

## Q5. Identity: how a remote member reaches the host and is verified

**The authority is the account service.** For a WebRTC connection:
1. The client calls `startRemoteSession(hostId)` → `{sessionId}`.
2. It generates a **fresh Ed25519 key per connection** (`team-webrtc-client-transport.ts:946-952`).
3. `POST /v2/remote/sessions/:id/ticket {clientPublicKey}` returns an **ES256 JWT** with audience `openbot-remote`. Its claims are `hostId, sessionId, userId, membershipId, role, authEpoch, sessionExpiresAt, clientPublicKey` (M:1080-1156; `packages/contracts/src/signal-protocol/ticket.ts:11-27`).

**The host verifies, in this order** (`team-webrtc-host-peer.ts:466-577`):
1. The JWT, with `jose.jwtVerify(ticket, JWKS, {audience, algorithms:["ES256"]})` (M:1201-1240).
   - The JWKS comes from the account service's own `/.well-known/jwks.json`. It is cached for the process lifetime and refetched when no key matches.
   - There is **no issuer check**.
2. The claims must equal Signal's `peer-ready` data, and `claims.clientPublicKey === frame.clientPublicKey` (`:481-493`).
3. The client's Ed25519 signature over this transcript (`:494-509`; `packages/contracts/src/team-protocol/webrtc-framing.ts:267-288`):
   `["openbot-team-v2-auth", hostId, sessionId, ticket, clientPublicKey, clientNonce, hostNonce, clientFingerprint, hostFingerprint]`
   The transcript includes **both DTLS sha-256 fingerprints**.
4. The host replies with `auth-ready`, signed with its device key. The client checks it against the **pinned** host key (`client-transport.ts:1372-1393`).
5. On `auth-complete`, `store.openRemoteSession` mints a random local token. It is held as a hash **in memory only**, together with the member and role *from the ticket claims* (`team-store.ts:975-1003`). A member the host doesn't know becomes `{id: membershipId, username: userId}`.
6. The Team API then authorizes routes by role and per-agent access level (`team-api-server.ts:2762-2781`).

**Host identity, seen from the client.**
- The pin is trust-on-first-use. The key comes from `devicePublicKey` in the account-service host list or invite preview, checked against the invite fingerprint (`remote-server-host-directory.ts:228-233`).
- Rotation goes through handovers signed by both the old and new keys (`team-store.ts:647-679`; `packages/contracts/src/host-key-handover.ts:18-28`).
- Mobile Connect's QR code carries the fingerprint out of band (`packages/contracts/src/mobile-connect.ts:19-31`, `:70-83`).

**Legacy HTTP path.** The host redeems an `accountTicket` at the account service (`redeemCentralTicket` → `/v1/team-auth/redeem`) and trusts the returned user's id and email (`team-api-server.ts:1200-1250`; M:1581-1609).

**Watch relay.** The ticket JWT plus an ECDSA P-256 proof. If the host doesn't know the member, it **builds the member from the ticket claims** (`watch-relay-service.ts:120`, `:235-245`).

**Consequences:**
- Whoever controls the account service or its ES256 key can mint a ticket for any user and role on any host. The account service can also impersonate a host on *first* contact.
- A malicious Signal or TURN server **cannot** man-in-the-middle the channel, because both signatures bind the DTLS fingerprints.
- There is **no content encryption beyond DTLS**, and no end-to-end encryption between people. The host holds everything in plaintext.

---

## What crosses the network (summary)

- **Joiner ↔ host, over DTLS data channels** `rpc`, `events`, `files`, and `desktop`, plus on-demand egress:
  - V2 JSON frames, at most 2 MiB each.
  - Team API requests and responses: agent rosters including the host `workspacePath`, full conversation messages, direct messages, presence, and files in 60 KiB chunks with a whole-file SHA-256.
  - Events with sequence numbers.
  - (part-webrtc Q1 and Q6; part-remote Q6)
- **Host or joiner ↔ account service, over TLS**:
  - profiles, member emails and roles, invites, which user connects to which host and when, host registration and keys;
  - push and Live Activity relays, integration route tickets;
  - the **GitHub token in the request body** (M:824-843).
- **Signal**: SDP, ICE, ticket metadata, integration deliveries, and sealed watch and queued deliveries.

## Code vs docs

- DOC AGENTS.md and the design notes describe the dispatch-order rules. The code matches them (`team-api-server.ts:1160-1256`).
- The design notes say the HTTPS V1/V3 and WebRTC V2 encodings are "released wire protocols." The code has separate codecs for each.
- The error strings "end-to-end authentication" (`team-webrtc-client-transport.ts:1336`, `:1411`) are backed by the handshake. "End to end" there means authentication, not encryption.

## Not read

- The server code: `apps/auth-api`, `remote/api` (except `tokens.ts:523-541`, read by the WebRTC pass), and `remote/coturn`.
- `push-notifications.ts`, `company-sign-in-browser.ts`, and the `team-api/route-*.ts` route bodies (except as cited).
- The renderer `team-webrtc.ts` beyond the parts read by the WebRTC pass.
- All tests.
