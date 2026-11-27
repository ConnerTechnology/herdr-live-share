# B (core, read by B lead): team-api-server, team-store, host-service, signal-ingress, host-signal-home, server-mode

Paths relative to `openbot/src/main/` unless noted. All claims below are from code unless marked DOC.

## Hosting

- Local Team API = plain Node `http.createServer` + `ws` WebSocketServer (`noServer`), bound to **127.0.0.1 only** on an OS-assigned port: `team-api-server.ts:720`, `:780` -> `listen-loopback.ts:19` (`server.listen(port, "127.0.0.1", ...)`). It is never exposed to the network directly. Remote clients reach it only through the WebRTC host gateway (`host-service.ts:1122-1128` passes `localApiPort: apiPort` to `gateway.start`).
- Host start sequence `host-service.ts:1069-1148` (`#startRuntimeOperation`):
  1. `assertOwnerAccount(signedInUser)` — only the owning account can publish (`:1085`).
  2. `api.start()` -> loopback port (`:1090`).
  3. `registerRemoteHost({hostId, name, ownerMembershipId, devicePublicKey: identity.publicKey})` to account service (`:991-1010`, wired to `centralAuth.registerRemoteHost`, application-services.ts:3585).
  4. In parallel: `listRemoteMembers(serverId)` -> `syncRemoteDirectory` (account service is the member directory) and `issueRemoteHostTicket(serverId)` -> `{ticket, signalUrl, expiresAt}` (`:1114-1120`).
  5. `gateway.start({hostId, signalUrl, ticket, localApiPort})` (WebRTC host gateway connects to Signal with the ticket).
  6. status `apiUrl: bootstrap.signalUrl` (`:1140`), `setEnabledOnLaunch(true)`.
- `startLoopbackOnly` (`:1181`) is dev-only: `apiUrl: http://localhost:<port>`.
- A desktop is a host iff `TeamStore` has an active `StoredTeam` for the signed-in account (`team-store.ts:378 configured`). `register()` auto-configures a server named "OpenBot" when none exists (`host-service.ts:1012-1016`). The file holds `hosts: StoredTeam[]` keyed by owner account, but only one is active at a time (`#state`, `team-store.ts:194`, `:286-291`).
- Server mode (headless Linux): `server-mode.ts:175-188` requires packaged + linux + `OPENBOT_SERVER=1`; control is an HTTP server on a unix socket `$XDG_RUNTIME_DIR/control.sock` with dir-ownership/0077 checks and chmod 0600 (`:240-277`). Routes: status, email-code login, server-claim login, rename, mobile-connect QR, logout (`:385-409`). `publish()` = configure + `host.start` (`:290-320`).
- Signal ingress (`signal-ingress.ts`): a second outbound WebSocket to the Signal service, peer `"ingress"`, authenticated by the same host ticket (`hello.token = bootstrap.ticket`, `:619-629`). Carries Slack/Discord/Telegram/webhook/GitHub/watch-relay deliveries in from public endpoints (`:710-865`). Optional "ingress queue": Signal holds deliveries sealed to a host `queueKey.publicKey`; host opens with `openQueuedDelivery(key.privateKey, hostId, sealed)` (`:673-694`). This is the one place in my files with sealed-at-relay data.
- Signal homes (`host-signal-home.ts`): list of regional Signal servers from the account service; host measures `/health/ready` latency and stores a preferred home in `openbot-signal-home-v1.json` (`:8`, `:46-71`, `:130-136`).

## Team API auth (host side)

- Gate order in `#handle` (`team-api-server.ts:1140-1263`): compatibility -> remote-screen -> protocol headers (426/400) -> unauthenticated routes (identity, invitation preview, join with username/password, join with account ticket, login, login with account) -> `Authorization: Bearer <token>` -> `store.authenticateSession(token)` else 401.
- WebSocket `/v1/events`: token carried as subprotocol `openbot-token.<token>` (`:747-749`), checked by `store.authenticate`. Heartbeat every 15 s re-authenticates each socket and closes on revocation (`:804-810`).
- Account-join path: body `accountTicket` -> `options.redeemCentralTicket(ticket, serverId)` -> `CentralAuthUser` (`:1200-1221`; wired to `centralAuth.redeemTeamAuthTicket`, application-services.ts:3651). The host trusts the account service's answer for identity (email, id).
- Sessions (`team-store.ts`): 32 random bytes base64url token, stored as SHA-256 hash, 30-day TTL (`:77`, `:1441-1461`, `:1584`). Remote (WebRTC) sessions are in-memory only: `openRemoteSession` mints a token keyed by hash in `#remoteSessions` (`:975-1003`), member identity from the stored directory row or `{id: membershipId, username: userId}`.
- Authorization: role (owner/admin/member) + per-agent access levels `none/…/chat/manage` (`team-api-server.ts:2762-2781`); hidden agents 404 (`:1262-1271`); events filtered per member (`:1866-2115`).
- Rate limit on sign-in: 5 attempts / 15 min per (source, username); source is `webrtc:<sessionId>` when header `x-openbot-webrtc-session` matches the bearer session (`:1665-1690`).

## Host identity key and data at rest (host)

- Each host has an **Ed25519** keypair generated at configure (`team-store.ts:442-447`, `:507-512`); `fingerprint = sha256(publicKeyPEM)` base64url (`:1558-1560`).
- Uses: `getIdentityProof(challenge)` signs a client challenge (`:416-421`, served at `GET identity?challenge=`, team-api-server.ts:1172-1179); `signRemoteAuthentication(transcript)` (`:423-426`, used by the WebRTC host side); key rotation handover signed by old and new keys, accepted by the account service (`:647-679`, host-service.ts:957-989).
- Storage: `userData/openbot-team-server-v2.json` (`team-store.ts:74`, application-services.ts:2817-2819) written with `JSON.stringify` via `writeFileAtomically`, file mode 0600, dir 0700 (`backend/atomic-json-file.ts:55,88`). **Private key PEM, member rows, invite token hashes, session token hashes, scrypt password hashes are all plaintext JSON — no encryption.**
- By contrast `RemoteServerManager` (the joined-servers list) is constructed with `safeStorageCipher(...)` (application-services.ts:3868-3870), i.e. encrypted (see remote/auth parts).

## Members and directory

- In production the member directory lives at the account service: `listRemoteMembers`, `createRemoteInvite`, `updateRemoteMember`, `removeRemoteMember` (host-service.ts:1320-1346, 1499-1598, 1833-1888). Local `syncRemoteDirectory` mirrors it (team-store.ts:839-917) and disables locals not present remotely. Local password invites (`acceptInvite`, `login`) are the dev/password-owner path (team-store.ts:1210-1276; host-service.ts:1249-1293).
- Invite URL embeds `{apiUrl: controlPlaneUrl, serverId, fingerprint, token}` (host-service.ts:1848-1857). Fingerprint lets the joiner pin the host key.

## Cross-host agent calls (Q3) from these files

- Backend (`src/backend`) never imports or references `RemoteServerManager` (grep over src/backend: no hits; only `automation-command.ts` builds a curl command against the *local* Team API with a local token file). Agents run in the backend against local runtimes; there is no agent tool that talks to another host.
- Cross-machine capability that does exist: `client-browser-v1` — host agents send `client-browser-command` events (open/navigate/click/type/screenshot…) over the member's event WebSocket to the joined member's browser (`team-api-server.ts:1011-1026`, contracts `team-protocol/client-browser-v1.ts`). That is a host agent driving a *person's* browser, not another host's agent. Also "browser egress" (joiner's computer as network proxy for the host browser; application-services.ts:1382, 3950-3956) and sign-in relays.
- Direct messages are person-to-person on one host (`team-api-server.ts:1076-1118`, `:2522-2560`).
