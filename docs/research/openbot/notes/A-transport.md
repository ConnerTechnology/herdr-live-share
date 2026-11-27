# A. Client-side team connection and wire transport (packages/team-client)

Reader A. Source: `openbot/` clone (read-only). Paths below are relative to the repo root.
`RP` = `packages/team-client/src/remote-peer.ts`, `RD` = `packages/team-client/src/remote-directory.ts`.

Scope note: `packages/team-client` is the shared client used by the **web client**
(`src/renderer/src/features/web-client/web-runtime.ts`, `web-host-connections.ts`), **mobile**
(`apps/mobile/.../remote-team-bridge.dom.tsx`, an Expo DOM component in a hidden WebView), the **iOS
share extension** (`apps/mobile/src/features/share/extension/share-sender.ts`) and the **browser
extension** (`apps/browser-extension/src/host-connection.ts`). The **desktop Electron client does NOT use
`createRemoteTeamPeer`**; it has a parallel Node implementation in `src/main/team-webrtc-client-transport.ts`
(1612 lines, `node:crypto`), which I did not read in full. The host side is `src/main/team-webrtc-host-*.ts`
(I read only `team-webrtc-host-peer.ts:465-545` to confirm the auth check).

---

## 1. End-to-end: discover host -> ticket -> signal -> connection

### Actors
- **Account service** ("control plane", Cloudflare Worker + D1, `api.openbot.run`). Directory, memberships,
  invitations, logical remote sessions, ES256 connection tickets. Client talks plain HTTPS+JSON, bearer token
  (desktop/mobile) or cookie (`/api/browser/*` + `X-OpenBot-Browser: 1`) — `RD:613-661`.
- **Signal** (Node service, `wss://signal{1,2,-us,-au,-sg}.openbot.run/v1/signal`) — WebSocket rendezvous that
  relays SDP/ICE, issues resume tokens and TURN credentials. Co-located **coturn** TURN relays
  (`turn*.openbot.run`) (`docs/remote-session-deployment.md:161-207,278-357`).
- **Host** = a user's desktop app (or OpenBot "hosted server" VM) that owns the agents. It keeps a Signal socket
  open (`peer: "host"`, `multiplex: true`).
- **Client** = a member device (web, phone, desktop-as-client, extension).

### Discovery (directory)
- `GET /v2/remote/hosts/` returns every host the account is a member of: `hostId, name, devicePublicKey (Ed25519
  SPKI PEM), membershipId, role, keyHandovers[], hosted, memberLimit, ...` (`RD:182-265`). Records without a
  `devicePublicKey` are dropped (`RD:191-192`).
- Host key is **pinned TOFU** per hostId in a `RemoteHostKeyStore` (`RD:562-588`): first sight stores the
  advertised key; later a different key is accepted only if a valid signed handover chain leads from the
  pinned key to it (`host-key-handover.ts:17-36`); otherwise the **old pinned key is silently kept**
  (`RD:583`), so the later auth handshake fails ("desktop identity not verified").
- Mobile-Connect pairing binds `{hostId, fingerprint}` out of band (QR); directory mismatch with no valid
  handover -> hard error `pairedIdentityChanged` (`RD:245-262`).
- Refresh: every 15 min while active (`RD:746-754`, `remote-account-refresh.ts`), plus Signal push
  invalidations `account-servers-changed` / `account-profile-changed` (`RP:1235-1242`). Docs:
  `team-api.md:101-106,222-228`.

### Invitations
- Invite URL = `{apiUrl, serverId, fingerprint, token}`; `fingerprint = base64url(sha256(hostPublicKeyPEM))`
  (`RD:297-317`, `host-key-handover.ts:12-15`). Created by an owner/admin client (`POST
  /v2/remote/hosts/:id/invites`) using the host key *it* has pinned.
- Joiner: `POST /v2/remote/invites/preview` (unauthenticated) -> checks preview `devicePublicKey` hashes to
  the URL fingerprint (or follows handovers) (`RD:497-530`), pins it, then `POST /v2/remote/invites/accept`
  (`RD:532-560`). Invite origin must equal the account API origin (`RD:500-508`).

### Session + ticket (bootstrap)
`RemoteTeamDirectoryClient.createBootstrap(hostId, clientPublicKey, existingSessionId)` (`RD:449-475`):
1. reuse a retained session if present (`POST /v2/remote/sessions/:id/ticket`), else
2. `POST /v2/remote/sessions/ {hostId}` -> `sessionId` (`RD:477-482`),
3. `POST /v2/remote/sessions/:id/ticket {clientPublicKey}` -> `{signalUrl, ticket}` (`RD:602-611`).

Ticket = ES256 JWT, claims (`packages/contracts/src/signal-protocol/ticket.ts:11-27`):
`aud:"openbot-remote", jti, sessionId, hostId, userId, membershipId, role, authEpoch, protocolMinimum/Maximum,
sessionExpiresAt, clientPublicKey, signal (home), iat, exp`. Ticket's `signal` claim names the Signal home
(`ticket.ts:29-43`, used at `RP:793`).

The client **Ed25519 identity is ephemeral**: a fresh keypair per `connectPeerEffect` (`RP:983`), never
persisted. Long-term client identity = the account (via ticket), not a device key.

### Signaling (Signal WebSocket, protocol v1, JSON text frames)
Messages: `packages/contracts/src/signal-protocol/messages.ts`.
- Client opens WS to `signalUrl`, sends `hello {version:1, peer:"client", token: resumeToken ?? ticket,
  waitForHost?}` (`RP:1151-1167`).
- Signal replies:
  - `ready {connectionId, resumeToken, iceServers[], reconnectGraceMs?, capabilities?}` (`RP:1303-1409`). ICE
    servers include TURN creds with 1 h TTL; `turn-refresh` every 45 min (`messages.ts:13-15`, `RP:2204-2222`).
    **Client hard-fails if `iceServers` is empty** (`RP:1330-1331`, "relayUnavailable").
  - `host-waiting {resumeToken}` when the host isn't connected; client stays, renews Signal every 60 s, and after
    10 s (doubling to 5 min) asks for a new ticket to follow the host to another Signal home (`RP:1268-1301`,
    `798-846`).
  - `error {code}` (`authentication_required|invalid_message|host_unavailable|host_busy|permission_denied|
    rate_limited|session_revoked|protocol_error`) (`RP:77-84,1243-1267`).
  - `disconnect {connectionId}` -> fail (`RP:1410-1413`).
- **Client is the offerer**. It creates the `RTCPeerConnection({iceServers, bundlePolicy:"max-bundle"})` and
  four **ordered, reliable** DataChannels, then `offer {connectionId, channel:"team", sdp}` (`RP:1386-1407`).
  Trickle ICE both ways via `ice-candidate` (`RP:1458-1471,1435-1443`). Host returns `answer` (`RP:1416-1434`).
- Data channels (`RP:417`, labels `contracts/team-protocol/webrtc-framing.ts:5-9`, `RP:2428-2439`):
  `openbot-team-v2-rpc`, `openbot-team-v2-events`, `openbot-team-v2-files`, `openbot.remote-desktop.signal.v1`
  (carries remote browser-view / remote-desktop stream control; `browser-view.ts`).
- Fast path: with ICE servers < 30 min old for the same Signal URL, the client pre-builds the PC + offer and
  pre-opens the WS while the ticket request runs; first offer carries gathered candidates (`RP:996-1083,
  1103-1117,1335-1368`; doc `team-api.md:53-55`).

### Authentication on the data channel (after all 4 channels open, `RP:1534-1538`)
Transcript (`contracts/team-protocol/webrtc-framing.ts:267-288`):
```
JSON.stringify(["openbot-team-v2-auth", hostId, sessionId, ticket, clientPublicKey,
                clientNonce, hostNonce ?? "", clientDtlsFingerprint, hostDtlsFingerprint])
```
Fingerprints are the `a=fingerprint:sha-256` lines of local/remote SDP (`RP:2441-2445`) => **channel binding to
the DTLS session**.
1. client -> `auth-init {ticket, clientPublicKey, clientNonce, signature=Ed25519(transcript w/o hostNonce)}`
   on rpc (`RP:1587-1621`).
2. host verifies ticket (ES256), checks claims match the Signal `peer-ready` (hostId, sessionId, userId,
   membershipId, role, sessionExpiresAt) **and** `claims.clientPublicKey === frame.clientPublicKey`, verifies the
   client signature, then -> `auth-ready {clientNonce, hostNonce, signature}` signed by the host device key over
   the transcript incl. hostNonce (`src/main/team-webrtc-host-peer.ts:476-532`).
3. client verifies with the **pinned** host PEM (`RP:1756-1779`), -> `auth-complete {clientNonce, hostNonce}`.
4. host -> `auth-confirmed`; client marks authenticated, sends `event-ack`, reports online (`RP:1795-1814`).
Any data on files/events/desktop before auth is a fatal error (`RP:1626-1644`).

### Sequence
```
Client                 Account API (CF)          Signal (+TURN)                Host
  | GET /v2/remote/hosts/  |                         |                           |
  |<-- hosts+devicePublicKey (TOFU pin) ------------|                           |
  | POST /v2/remote/sessions {hostId} -> sessionId  |                           |
  | POST .../ticket {clientPublicKey(ephemeral Ed25519)} -> {signalUrl, ES256 ticket}
  |                        |                         |<== host WS (peer:host, multiplex) kept open
  |== WS hello{peer:client, token:ticket} ==========>|                           |
  |<== ready{connectionId,resumeToken,iceServers} ===|-- peer-ready{session,user,role} -->|
  |-- offer(sdp, 4 DataChannels) =================>|-- offer ------------------>|
  |<===================================== answer --|<-- answer ----------------|
  |<== ice-candidate <==> ice-candidate (trickle) ===|<========================>|
  |================= ICE / DTLS (direct or via TURN relay) =====================|
  | rpc: auth-init{ticket, clientPub, nonceC, sigC} ------------------------------>| verify ES256 ticket,
  |<-------------------------------- auth-ready{nonceH, sigH(host device key)} ----| claims==peer-ready,
  | verify sigH with PINNED host key                                              | clientPub==ticket
  | rpc: auth-complete ----------------------------------------------------------->|
  |<------------------------------------------------------------- auth-confirmed --|
  | rpc: http.request GET /v1/compatibility (choose protocol) -------------------->|
  | events: event-ack / event-control(runtime-snapshot-request)                    |
```
TURN: Signal always hands out TURN servers; whether the path is relayed is decided by ICE (`RP:734-742`
`usesRelay`; a relayed path is re-ICE'd on each TURN refresh to try for a direct path, `RP:1321-1322`).
Docs: host "private" route preference drops non-private candidates and gives the host PC no ICE servers
(`team-api.md:13-19`) — I did not see that filtering in team-client; it lives in the host's hidden page.

---

## 2. Framing / protocol on the data channel

### Transport framing (`packages/team-client/src/webrtc-framing.ts`, "V2", frozen — `AGENTS.md:13-16`)
- A text payload that fits in SCTP `maxMessageSize` is sent as a plain string (`webrtc-framing.ts:30`).
- Otherwise (or any binary) it is chunked: 17-byte header `[magic 0x4F425732 "OBW2"][u32 messageId][u32 total]
  [u32 offset][u8 type 1=text 2=binary]` + chunk; frame <= 64 KiB; reassembled payload <= 2 MiB; 16 concurrent
  assemblies, 8 MiB buffered, 30 s expiry; contiguous only (`webrtc-framing.ts:7-135`, limits in
  `contracts/team-protocol/webrtc-framing.ts:10-15`).
- Backpressure: wait if `bufferedAmount > 4 MiB` until `bufferedamountlow` (1 MiB); fail if no drain in 60 s
  (`RP:1529-1530,2080-2136`).

### Frame types (JSON, all `version: 2`) — `contracts/team-protocol/webrtc-framing.ts:32-76`
- **rpc channel**: `request {requestId, operation, payload}` / `response {requestId, result|error{code,message,
  retryable,status?}}`; plus the 4 auth frames.
- **events channel**: `event {sequence, payload}`, `event-ack {throughSequence}`, `event-reset {nextSequence}`,
  `event-control {control: runtime-snapshot-request | team-typing{botId,typing} | team-direct-typing{
  recipientMemberId,typing} | workflow spot}`.
- **files channel**: control `file-open {transferId,name,size,mimeType,sha256}`, `file-ack {receivedThrough}`,
  `file-complete`, `file-cancel {reason}`; binary chunk `[u8 2][u8 idLen][transferId][u64 offset][bytes]`
  (`contracts/.../webrtc-framing.ts:290-327`).

### RPC = HTTP tunnelled over the data channel
Only operation used is `"http.request"` (`RP:1877-1891`):
```
{version:2,type:"request",requestId:<16 random bytes hex>,operation:"http.request",
 payload:{method, path:"/v1/...", body|null, bodyTransferId?, contentType?, capabilities:[...], absent:[...]}}
```
Response `result: {status, body}` or `result: {status, file:{transferId}}` for downloads (`RP:1645-1704`).
The routes are the host's Team API REST routes (`TEAM_API_ROUTES`, and dozens of `*-v1` capability route sets
used in `team-api-requests.ts` and `team-admin-requests.ts`). Timeouts: default 10.5 min, extended while the
host keeps answering other requests (`RP:428`, `remote-recovery.ts:427-444`); workspace reads 15 s
(`remote-recovery.ts:446-460`).

### Versioning
- Start of each connection: `GET /v1/compatibility` -> `{app version, protocol min/max, capabilities}`; client
  picks highest shared protocol (`team-api.md:306`, `RP:1669-1673`). Current: v9/v10; v1 frozen; public
  HTTP/WebSocket/Cloudflare-Tunnel transport retired (`team-api.md:5,303-304`).
- On WebRTC no protocol header: protocol inferred from capability markers in each request
  (`long-capability-lists-v1`, `capability-baseline-v1`, `teamCapabilityLists`) (`team-api.md:317-347`,
  `RP:1872-1888`). Optional features are capability-gated per request; unknown optional events ignored,
  malformed known events => `protocol_error` (`team-api.md:372`).

### Event stream
- Strictly sequenced. Duplicate (<= last) is acked and dropped; a gap is fatal (`eventStreamGap`) -> reconnect
  (`RP:1722-1731`). `event-reset` from the host means "you missed events, reload" -> `resync` (`RP:1708-1721`).
- Ack debounced 16 ms (`RP:2055-2062`). On a *new* connection `lastEventSequence` starts at 0 (`RP:1036`).
- Events decode to `AgentEvent | TeamRealtimeEvent | LiveVoiceEvent` (`RP:110,1732-1749`): conversation
  snapshots/deltas, turn-started/progress/completed, prompt, approval, queue-changed, runtime-snapshot,
  agents-changed, browser-takeover, etc. (consumers in `dynamic-island-coordinator.ts:149-277`).

### Files
- Upload (`file-upload.ts`): one transfer at a time; `file-open` with sha256 of whole file, wait for host
  `file-ack(0)`, stream 60 KiB chunks (16-chunk parts read from device), `file-complete`, wait until host acked
  all bytes; 60 s idle stall timeout. Then the RPC request carries `bodyTransferId` instead of a body
  (`RP:1849,1887`). Limit 100 MB/file protocol (`contracts .../webrtc-framing.ts:10`), 10 MB default for mobile
  base64 bridge (`file-upload.ts:23,156`).
- Download (`file-download.ts`): host sends `file-open` on files channel; client acks per chunk, hashes
  incrementally, verifies sha256 on `file-complete`; > 10 MB only with an explicit allowance from the request;
  max 10 concurrent, 20 MB total unallowed (`file-download.ts:208-249,277-301`).
- Larger artifacts use chunked app-level routes over the same RPC (agent import v2, instance transfer chunks
  with per-chunk sha256) (`team-api-requests.ts:499-516,683-725`).

---

## 3. Crypto and the trust model

### What the code does
| Mechanism | Where | Purpose |
| --- | --- | --- |
| DTLS (WebRTC built-in) | browser/Node WebRTC | Only payload encryption on the data channels. **No app-level E2E encryption layer on team traffic.** |
| Ed25519 host device key (long-term, PEM SPKI) | `ed25519.ts` (@noble/ed25519, `zip215:false`), host store | Host signs auth transcript; client verifies with pinned key. |
| Ed25519 client key (ephemeral per connection) | `RP:983` | Proof of possession bound into ticket (`clientPublicKey` claim) and transcript. |
| DTLS fingerprint channel binding | `RP:1589-1604,2441-2445` | Both SDP fingerprints are inside the signed transcript, so Signal cannot MITM by swapping SDP. On ICE restart the host's fingerprint must not change (`RP:1417-1424`). |
| ES256 ticket (account service) | `signal-protocol/ticket.ts` | Authorizes {user, membership, role, session, clientPublicKey} to the host. Host verifies it (`team-webrtc-host-peer.ts:476-491`). |
| Host key handover | `host-key-handover.ts`; statement `["openbot-host-key-handover-v1",hostId,issuedAt,prevPub,pub]` (`contracts/src/host-key-handover.ts:18-28`) | Key rotation (admin route `HOST_KEY_ROTATION_ROUTES.rotate`, `team-admin-requests.ts:901-910`): each step must be signed by **both** previous and next key. Client walks the chain from its pinned fingerprint. |
| Live-activity / push "seal" | `live-activity-seal.ts`, `push-notification-seal.ts` | Hand-rolled AEAD: keystream `HMAC-SHA256(sealKey, nonce16 ‖ u32 block)` XOR plaintext, tag = first 16 B of `HMAC-SHA256(tagKey, nonce ‖ ct)` (encrypt-then-MAC). Keys = HMAC(secret, "openbot live activity seal/tag/action" or "openbot push notification seal/tag"). Phone has a 32-byte secret in SecureStore; per-host secret = `HMAC(phoneSecret, "openbot live activity host "+serverId)` (`live-activity-seal.ts:16-24`; `apps/mobile/.../live-activity-keys.ts:41`). Host seals notification JSON `{serverId,agentId,threadId,kind,title,body}` before sending to OpenBot's push relay -> APNs/FCM (`src/main/push-notifications.ts:396-527,647-656`), so the relay/Apple/Google see only token + opaque blob. `signLiveActivityLink`/`verifyLiveActivityLink` (HMAC-16B, constant-time compare) authenticate lock-screen action deep links (approve/answer) so other apps can't forge them (`live-activity-props.ts:442-472`). |
| Watch relay | `docs/architecture/watch-relay.md` | The **only** relay path with app-level E2E: watch -> Signal HTTPS -> host ingress WS, ECDH P-256 + HKDF + AES-256-GCM, ECDSA P-256 proof, ES256 watch ticket (`watch-relay.md:25-32,156-171`). Signal sees metadata only. |
| Workspace prefs key hashing | `workspace-preferences.ts:31-32` | Local storage keys are sha256 of (origin,userId)/hostId — privacy of local storage, not crypto security. |

### What OpenBot's servers can see / tamper with (inferred from code; host internals belong to another reader)
- **Account service**: sees directory/membership graph, invites, session start/end, ticket issue (incl. client
  ephemeral public keys), IPs, profile. It is the **authority for who is a member and with what role**: the host
  accepts any validly signed ticket whose claims match Signal's `peer-ready`. So OpenBot's control plane (or
  whoever holds its ES256 key) **can mint a ticket and impersonate any member to a host**; nothing in the client
  or the host-auth code I read requires a per-member key the host has independently pinned. The watch doc even
  says a member the host doesn't know yet is accepted "as for a remote session" (`watch-relay.md:76-78`).
- **Host authenticity**: the client trusts only the pinned host Ed25519 key, so the control plane **cannot
  impersonate a host after first pin** (and a key swap needs a dual-signed handover). But the **first pin of a
  directory-listed host is TOFU from the account service** (`RD:570-572`); only Mobile-Connect QR and invite
  URLs carry an out-of-band fingerprint (and an invite fingerprint is itself derived from the inviter's
  TOFU-pinned key, `RD:297-301`). Web client: "Host fingerprints are checked before use" (`web-client.md:75-76`).
- **Signal**: sees SDP (ICE candidates => IP addresses, DTLS fingerprints), connection timing, who connects to
  which host. Typing/presence controls go over the data channel, not Signal. It cannot read or alter
  data-channel content (DTLS + fingerprint-bound signatures). It can deny service, delay, or drop. It forwards
  account notifications and the host's Slack/Telegram/Discord/GitHub/webhook ingress (those are server-visible
  by design; `messages.ts:96-219`; `queued-delivery` carries a `sealed` field — not studied here).
- **TURN (coturn)**: relays DTLS ciphertext; sees endpoints and volume.
- Docs claim (`team-api.md:254-301`) session revocation propagates via D1 outbox -> Signal `sessions-ended` ->
  host closes child peers; host re-confirms peers after Signal reconnect within 11 min. Not verified here.

---

## 4. Recovery, reconnect, presence, multi-server, host-as-client

### Recovery inside the peer (`RP`)
- Two layers: (a) **ICE-level recovery on the same PeerConnection** (keeps channels, decoders, event sequence),
  (b) **replace the PeerConnection on the same logical session** (`closePeer(..., retainSession=true)` keeps
  `retainedSession`, next bootstrap reuses it, `RP:2326-2356,991`).
- `disconnected` -> ICE restart after 2 s (immediate on `failed`), probe 6 s, Signal renew after 8 s, give up after
  15 s grace (`RP:418-421,760-778,848-856,1497-1524`). Client counts ICE restarts and replaces the connection
  instead of an 11th (host closes at 11) (`RP:441,2138-2169`).
- Signal socket loss with a healthy DataChannel is fine: WebRTC continues; client reconnects Signal with
  `resumeToken`, backoff `500ms*2^n` max 30 s with jitter (`RP:429-432,2181-2198`). 1008 / `rate_limited` -> 60 s
  hold (`RP:1199-1202,1246-1261`). After 2 failed opens with a home-pinned ticket, or 10 s+ of host-waiting, fetch a
  new ticket (Signal home may have moved) (`RP:791-846`; `remote-session-deployment.md:246-252`).
- Connect deadline 30 s with staged diagnosis signal/ice/channels/auth (`RP:2273-2288,2377-2384`).
- Liveness probe: the compatibility GET is used as a path check that inspects RTCStats bytesSent/Received to
  decide "send-stalled" vs "no-answer" within 1.5-12 s (`remote-path-check.ts`, `RP:1893-1937`). Any other
  request timeout triggers such a probe (`RP:2004-2013`).
- App background/foreground (`setActive`, `RP:613-664`): background cancels pending GETs and marks resync;
  return after < 30 s keeps everything; >= 30 s renews Signal + ICE restart (ICE consent expired); >=
  `120 s + reconnectGrace + 10 s` replaces the connection (`RP:433-440,646-662`). `networkMoved` (Wi-Fi<->cell)
  replaces at once (`RP:676-681`).
- `kept: true` on `connect` result tells the UI the old connection and event stream survived, so it only checks
  compatibility and reloads open chats (`RP:895-910`; `team-api.md:33-37`).
- App-level controller `createRemoteConnectionRecovery` (`remote-recovery.ts:173-373`): 5 attempts at 2 s
  exponential, then 30 s cooldown; suspend on protocol errors until manual Retry.
- Hosted (OpenBot-run VM) servers: client can call wake/status endpoints and wait up to 15 min
  (`hosted-server-wake.ts`).

### Presence / typing
- `setTyping(agentId, typing)` -> `event-control team-typing` (refresh 2 s, deduped) (`RP:571-595`); wire also has
  `team-direct-typing {recipientMemberId}` (member-to-member DM typing).
- `setWorkflowSpot` -> workflow canvas presence (`RP:596-600`, `workflow-presence-v1`).
- Focus ping per server at `FOCUS_USE_INTERVAL_MS` (`focus-use.ts`).
- Presence of a *person* is derived by the host from connected peers: "Each status connection uses one Signal
  socket, and the host shows the user as present" (`web-client.md:79-86`).

### Multiple servers per client
- One `createRemoteTeamPeer` instance holds **one** `peer` at a time (`RP:504`); `connect` to a different host
  closes the current one (`RP:903-917`). Multi-server = one peer instance per server: mobile keeps "its own
  transport and recovery controller" per membership, selected server first, others after it is online or 3 s
  (`team-api.md:20-24`); web client keeps one host per tab (Web Lock) plus lightweight "status connections"
  (connect + compatibility, no events) for the other hosts, coordinated over `BroadcastChannel`
  (`web-client.md:77-86`).
- Host multiplexes many client peers (opt-in `hello.multiplex`), one child RTC peer per device/session; legacy
  hosts allowed one client (`team-api.md:269-273`; `remote-session-deployment.md:1-17`).

### Can a host also be a client of another host?
- Yes, at the **app** level: the desktop app both publishes its own server and joins others ("Only the paired
  desktop is Local; other servers are Remote", `team-api.md:26-27`; "the desktop message menu on a joined
  server", `team-api.md:168-169`; desktop client = `src/main/team-webrtc-client-transport.ts`). A host can even
  run several "local servers" (`LOCAL_SERVERS_ROUTES` list/create/delete, `team-admin-requests.ts:912-959`).
- But these are **separate roles**: nothing in team-client bridges a host's agents into the client connection.
  The joining desktop is just a UI client of the remote host.

---

## 5. Agent-to-agent across hosts?

**No evidence of it in team-client; the design points the other way.**
- Every request the client can make is a REST route to the connected host's own agents/channels
  (`team-api-requests.ts`, `team-admin-requests.ts`, ~300 wrappers, all `request(method, "/v1/...")`). None
  takes a target server, peer, or remote agent address.
- The only identity the client holds is the account + an ephemeral key; agents on the host never get a ticket,
  directory client, or `createRemoteTeamPeer` (instantiated only in web runtime, mobile DOM bridge, share
  extension, browser extension: grep of `createRemoteTeamPeer`).
- Agent-to-agent traffic that does exist is **intra-host**: messages with `senderAgentId`
  (`dynamic-island-coordinator.ts:607`), `turn-completed.origin === "agent"` ("teammate" turns,
  `agent-notifications.ts:153-155`), `agent-thread-requests-v1` = "requests that the agent sent from that
  thread to other agents" on the same host (`team-api.md:676-686`), channels where several agents of one host
  talk (`teamChannelsApi`, `team-api-requests.ts:859-903`).
- Cross-person collaboration is therefore: **many people join ONE host and talk to that host's agents** (and
  to each other in channels / DMs on that host). Person B's own agents on B's host are not reachable from A's
  host's agents through this transport. (Whether the host side has some other federation is for the host
  readers; nothing on the wire protocol here supports it.)
- Instance transfer / agent import (`team-api-requests.ts:476-775`) moves *copies* of agents between hosts via the
  client (export zip from one, upload to another) — data movement, not live agent RPC.

---

## Doc claims vs code (spot checks)
- Doc: Signal carries only SDP/ICE except Slack (`team-api.md:7-8`). Code: Signal messages also carry
  Telegram/Discord/GitHub/webhook/watch deliveries and account invalidations (`messages.ts:96-219`); client ignores
  webhook/github frames (`RP:1228-1234`). Mostly consistent (those go to the host socket).
- Doc: private route -> no ICE servers. Code: client errors on empty `iceServers` (`RP:1330`); so either Signal
  still sends them to clients and only the host side drops them, or the doc describes host-side config only.
  Unverified.
- Doc: protocol v1 HTTP/WS retired; code only has WebRTC in team-client (consistent).
- Doc v10 two capability lists; code sends `teamCapabilityLists(capabilityProtocol, ...)` (`RP:1888`)
  (consistent, implementation in contracts not read).

## Files read
In full: every file in `packages/team-client/src/` (52 .ts files, no test files exist in that dir),
`packages/team-client/AGENTS.md`, `packages/team-client/package.json`, `docs/architecture/team-api.md`,
`docs/architecture/watch-relay.md`, `docs/web-client.md`, `docs/remote-session-deployment.md`,
`packages/contracts/src/signal-protocol/messages.ts`.
Partially: `packages/contracts/src/team-protocol/webrtc-framing.ts` (types, transcript, file chunk codec),
`packages/contracts/src/signal-protocol/ticket.ts` (claims), `packages/contracts/src/host-key-handover.ts`
(statement), `src/main/team-webrtc-host-peer.ts:465-545`, greps of `src/main/push-notifications.ts` and
`apps/mobile/.../live-activity-keys.ts`.
Not read: desktop client `src/main/team-webrtc-client-transport.ts`, host gateway, Signal service, account Worker.
