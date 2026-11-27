# OpenBot: Team WebRTC transport (src/main/team-webrtc-*.ts)

Read-only source study. Nothing was run, installed or built. All paths are relative to the repo root
`openbot-src/openbot/`. The note marks claims that come from comments or error strings separately from what
the code does. These files have almost no explanatory comments. Most claims below come from the code
itself.

## Files

**Read in full, top to bottom**
- `src/main/team-webrtc-bridge.ts` (505 lines)
- `src/main/team-webrtc-client-transport.ts` (1612 lines)
- `src/main/team-webrtc-file-transfer.ts` (934 lines)
- `src/main/team-webrtc-host-gateway.ts` (566 lines)
- `src/main/team-webrtc-host-peer.ts` (1371 lines)
- `packages/contracts/src/team-protocol/webrtc-framing.ts` (391 lines). This is the "V2" frame codec.
  `team-protocol/v2.ts` is a one-line re-export of it.
- `packages/contracts/src/signal-protocol/ticket.ts` (43 lines)
- `src/preload/team-webrtc.ts` (7 lines)

**Followed partly, only the parts that answer a question**
- `src/renderer/src/features/team/team-webrtc.ts`: lines 1-60, 215-240 and 329-848. This hidden renderer
  owns the RTCPeerConnection, the Signal WebSocket, the data channels and the fingerprint extraction.
- `packages/contracts/src/signal-protocol/messages.ts`: lines 75-175 (Signal message shapes)
- `packages/team-client/src/webrtc-framing.ts`: lines 23-63 (fragmentation of data-channel payloads)
- `src/main/central-auth-manager.ts`: lines 1122-1240 (ticket JWT verification, JWKS)
- `src/main/team-store.ts`: lines 410-430 and 975-1012 (host signing key, local remote-session tokens)
- `src/main/team-api-server.ts`: lines 1670-1700. I also grepped it for `listenLoopback`.
- `src/main/listen-loopback.ts`: head of the file
- `src/main/remote-server-manager.ts`: lines 595-615 and 2715-2735 (where host keys get pinned)
- `src/main/remote-server-host-directory.ts`: lines 85-135 and 228-241 (TOFU and key handover)
- `src/main/remote-screen-gateway.ts`: lines 1291-1296. `src/main/browser-view-gateway.ts`: lines 270-278.
- `src/main/browser-egress-host.ts`: lines 1-80
- `packages/contracts/src/team-protocol/browser-egress-v1.ts`: head of the file
- `remote/api/src/tokens.ts`: lines 523-541 (how the server builds STUN/TURN entries)

**Skipped**
- All `*.test.ts` files
- The rest of the renderer `team-webrtc.ts` (reconnect, TURN refresh and private-network timers)
- `host-service.ts`, `server-safe-mode.ts` and `application-services.ts`. I only grepped them for wiring.
- I did not open `signal-ingress.ts` or `host-signal-home.ts`.

---

## Q1. What the transport is for: Team API HTTP tunnelled over data channels

**Yes. It is an "HTTP over data channel" tunnel** from a joining client's desktop app to a host's local Team
API server. Real-time events stream on a separate channel, and files and binary bodies go on a third.

### Process layout
- Main-process code never touches WebRTC directly. `TeamWebRtcBridge` creates a hidden, sandboxed
  BrowserWindow that loads `openbot-app://app/team-webrtc.html`. It talks to that window over a
  `MessageChannelMain` port (`team-webrtc-bridge.ts:284-350`). The window is created with `sandbox: true`,
  `contextIsolation: true`, `nodeIntegration: false` and `webSecurity: true` (`:287-296`).
- Bridge commands are `connect`, `prepare-signal`, `send`, `disconnect`, `disconnect-peer`, `restart-ice`,
  `open-egress`, `send-egress` and a few others (`team-webrtc-bridge.ts:99-125`). Each command waits for a
  `command-complete` reply, with a 15 s timeout, or 75 s for `send` (`:127-128`, `:389-422`).
- A liveness ping runs every 30 s. After 2 misses the bridge force-crashes the renderer, unless it shares
  its process with another window (`:352-387`).
- There are four logical channels: `"rpc" | "events" | "files" | "desktop"` (`team-webrtc-bridge.ts:14`).
  Their data-channel labels are `openbot-team-v2-rpc`, `openbot-team-v2-events` and `openbot-team-v2-files`
  (`webrtc-framing.ts:5-9`), plus `openbot.remote-desktop.signal.v1` (renderer `team-webrtc.ts:723`). All
  are `ordered: true` (`team-webrtc.ts:724`).
- A fifth channel, `openbot.browser-egress.v1`, is opened on demand by the client (`team-webrtc.ts:227`).
  It carries a TCP-stream mux: control frames `open {host, port}`, `opened`, `credit` and `close`
  (`browser-egress-v1.ts:12-17`). `BrowserEgressHost.exit(membershipId)` (`browser-egress-host.ts:55-66`)
  shows the direction. **The host's agent browser uses the connected member's network as its exit.** The
  host asks the client to open outbound TCP streams.

### Framing: the "V2" encoding (`packages/contracts/src/team-protocol/webrtc-framing.ts`)
- Every text frame is JSON with `version: 2` and a `type`. There are four families:
  - **RPC** (`:32-35`): `request {requestId, operation, payload}` and
    `response {requestId, result | error{code, message, retryable, status}}`
  - **Auth** (`:37-48`): `auth-init`, `auth-ready`, `auth-complete` and `auth-confirmed`
  - **Event** (`:50-62`): `event {sequence, payload}`, `event-ack {throughSequence}`,
    `event-reset {nextSequence}`, and `event-control {runtime-snapshot-request | team-typing |
    team-direct-typing | workflow spot}`
  - **File control** (`:64-76`): `file-open {transferId, name, size, mimeType, sha256}`,
    `file-ack {receivedThrough}`, `file-complete` and `file-cancel {reason}`
- Validators are strict:
  - IDs must match `/^[A-Za-z0-9:_-]{1,128}$/` (`:345-347`), and operations must match
    `/^[a-z0-9][a-z0-9._:/-]{0,127}$/` (`:349-351`).
  - JSON depth is at most 32, with at most 10k array items or object keys (`:369-376`).
  - A JSON frame is at most 2 MiB (`:13`, `:263`, `:380`).
  - `encodeTeamProtocolV2Frame` re-decodes every frame before it stringifies it (`:242-265`).
- Binary file chunks have this layout (`:290-327`):
  `[0]=2 | [1]=idLen | transferId | u64 offset (big-endian) | bytes`. A chunk frame is at most 64 KiB (`:12`).
- Below the V2 frames there is a fragmentation layer, `packages/team-client/src/webrtc-framing.ts:23-56`.
  A message larger than SCTP `maxMessageSize` is split into binary fragments with a 16-byte header:
  `magic u32 | messageId u32 | totalLen u32 | offset u32` plus a type byte (text or binary). The fragmented
  total is capped at 2 MiB. The renderer reassembles fragments with `TeamWebRtcPayloadDecoder`
  (`team-webrtc.ts:736-757`). Backpressure: `waitForWritableChannel` waits for the `bufferedamountlow`
  event and gives up after 60 s (`team-webrtc.ts:815-848`).
- "Protocol versions" v4 through v10 are *payload adapters* layered on the V2 frames. They are not new
  framings. The client encodes bodies with `encodeTeamProtocolV9WebRtcHttpRequest` and decodes responses
  with `decodeTeamProtocolV10WebRtcHttpResponse` (`team-webrtc-client-transport.ts:30-32`, `:559`, `:620`).
  The host picks a decoder for v4 through v9 and later, based on the capability strings the client sends
  (`team-webrtc-host-peer.ts:760-771`, `:1256-1272`). Capability lists ride inside each request payload
  (`client-transport.ts:565`, `host-peer.ts:715-731`).

### Request/response
- Client (`team-webrtc-client-transport.ts:489-644`):
  - It builds `{version:2, type:"request", requestId: randomUUID, operation:"http.request",
    payload:{method, path, body, capabilities…, bodyTransferId?, contentType?}}` (`:541-570`).
  - It sends the request on `"rpc"` (`:705`) and waits on a `#pending` map keyed by requestId (`:667-730`).
  - The default timeout is 10.5 min (`:58`). The deadline is extended while the host keeps answering
    (`startRemoteRequestDeadline` with `#lastAnswerAt`, `:688-698`).
  - A binary body is first uploaded as a file transfer, and the request then carries `bodyTransferId`
    (`:525`, `:532-539`).
  - A response may instead carry `file: {transferId}`, which the client consumes from the file transport
    (`:598-600`).
  - If the channel closed mid-request, the client reconnects and retries once with the same requestId
    (`:575-584`).
  - It learns the host's protocol by calling `GET /v1/compatibility` first (`:518`, `:646-665`, `:637-640`).
- Host (`team-webrtc-host-peer.ts:579-626`):
  - It decodes the request (`:586-594`).
  - **It replays a cached response for a duplicate requestId.** The cache only holds non-GET responses,
    up to 1000 entries or 4 Mi characters (`:595-599`, `:609-611`, `:157-158`, `:628-647`).
  - It de-duplicates concurrent in-flight requests per session (`:600-622`), then forwards the request
    to loopback (see Q4).
  - It rejects any operation other than `http.request` with 400 (`:659-663`).
  - Errors map to `GatewayError` status and code, with 413 `file_too_large` and 500 `host_error`
    (`:669-692`). A response that is too large to encode becomes 413 `response_too_large` (`:1313-1329`).

### Event streaming
- The host opens a local WebSocket to `ws://127.0.0.1:<port>/v1/events`, authenticating with subprotocols
  `["openbot-team-v1", "openbot-token.<localSessionToken>"]` (`host-peer.ts:905-911`).
- Each local event is re-encoded for the peer's capability level, wrapped as `{type:"event", sequence}`
  (`:917-971`) and buffered: at most 2000 events or 32 Mi characters, and the buffer is cleared on
  overflow (`:155-156`, `:976-984`).
- `#sendEvents` drains the buffer in order. A gap or send failure produces `event-reset {nextSequence}`
  (`:1041-1073`).
- The client requires strictly consecutive sequences. A gap causes `failProtocol` and a disconnect
  (`client-transport.ts:1480-1488`). Duplicates are only acknowledged again. The client acks in batches
  every 16 ms (`:1511-1527`). On `event-reset` it asks for `runtime-snapshot-request` (`:1462-1474`).
- Note: the host **ignores `event-ack` contents**. `#handleEventControl` returns on `event-ack` without
  using `throughSequence` (`host-peer.ts:1037-1038`). The buffer deletes each event when it is sent
  (`:1054-1056`). Delivery relies on SCTP ordered reliability, with no retransmission from the ack.
- Client→host `event-control` frames (typing, snapshot request, workflow spot) are passed straight into
  the local events WebSocket (`host-peer.ts:1027-1035`).

### Desktop channel
- The client sends control frames `open {streamId, path}`, `text` and `close`, plus binary frames.
- The host opens a local WebSocket only to `/v1/remote-screen/sessions/<id>/stream` or a browser-view
  stream path, with at most 6 streams. It sends only the header `X-OpenBot-WebRTC-Session: <sessionId>`
  and **no bearer token** (`host-peer.ts:1126-1155`).
- The local gateways accept that header alone when it equals the screen or view session's `teamSessionId`
  (`remote-screen-gateway.ts:1291-1293`, `browser-view-gateway.ts:270-272`). Any local process that knows
  a session id could present it. That is a loopback-trust shortcut, not a network exposure.

---

## Q2. Signaling, host addressing, STUN/TURN

- **Signaling goes through OpenBot's "Signal" service over a WebSocket.**
  - The `signalUrl` and a `ticket` come from the account service's `issueTicket(sessionId,
    clientPublicKey)` (`client-transport.ts:967`, `:1012`, `:1117-1121`).
  - The host gets its own `signalUrl` and ticket from `renewSignal` and the gateway `start` input
    (`host-gateway.ts:150-183`, `:543-565`).
  - The renderer opens `new WebSocket(signalUrl)` and sends `hello {version:1, peer:"host"|"client",
    token: resumeToken ?? ticket, multiplex (host only), waitForHost?}` (`team-webrtc.ts:339-351`;
    message shape at `messages.ts:83-98`).
  - The server answers `ready {connectionId, resumeToken, iceServers, …}` (`messages.ts:140-148`). For a
    host, it also sends `peer-ready {connectionId, sessionId, userId, membershipId, role, sessionExpiresAt,
    resumed}` (`messages.ts:151-161`).
  - Offers, answers and ICE candidates are relayed as `{type: "offer"|"answer"|"ice-candidate",
    connectionId, channel:"team", sdp|candidate}` (`team-webrtc.ts:506-512`, `:610-646`, `:657-669`).
  - The **client is the offerer** and creates all four data channels (`team-webrtc.ts:498-512`). The host
    answers (`:610-629`).
  - One host Signal socket multiplexes many clients. Relayed frames are routed to a per-client sub-state
    by `connectionId` (`team-webrtc.ts:516-583`).
  - The same Signal socket also carries unrelated relays: Slack, Discord, Telegram, GitHub, webhook,
    queued and watch deliveries (`team-webrtc.ts:401-414`, `messages.ts:83-135`).
- **Host addressing.** The client addresses the host by `hostId`, which is also used as the bridge
  `peerId` (`client-transport.ts:1117-1118`). The Signal service resolves the host from the ticket claims
  (`RemoteTicketClaims.hostId`, `ticket.ts:11-27`).
  - On the host, each incoming client gets a random `peerId` (`team-webrtc.ts:526`). The host's own Signal
    socket uses `peerId = hostId` (`host-gateway.ts:171`).
  - The gateway ignores an `incoming` event whose `peerId === hostId` (`host-gateway.ts:354`).
  - A ticket may name a "Signal home" in its `signal` claim (`ticket.ts:25`, `:29-43`). The gateway moves
    between Signal homes with backoff: from 20 s up to 10 min, a 15-min return check, and a "lost" report
    after 3 min (`host-gateway.ts:25-29`, `:225-302`, `:442-499`).
  - Terminal codes are `session_revoked`, `authentication_required`, `permission_denied` (except a stale
    relay) and `host_busy`. Close code 4000 is also terminal (`team-webrtc.ts:438-455`, `:386-392`).
- **STUN/TURN.**
  - ICE servers are **only those the Signal service sends in `ready.iceServers`** (`team-webrtc.ts:474-493`).
    No STUN or TURN URLs are hard-coded in the client (I grepped for `stun:`, `turn:` and `turns:`).
  - Server side (`remote/api/src/tokens.ts:523-541`): `stun:<host>:<port>` plus
    `turn:<host>:<port>?transport=udp|tcp` and `turns:<host>:<tlsPort>?transport=tcp`. TURN uses the
    time-limited REST credential scheme: `username = "<exp>:<sessionId>"` and
    `credential = HMAC-SHA1(turnSecret, username)`.
  - The TURN host is chosen per hostId. The repo ships a `remote/coturn/` deployment.
  - `iceTransportPolicy` can be forced to `"relay"` (`team-webrtc-bridge.ts:52`, `:178`).
  - "Private network" mode sets `iceServers: []` and filters candidates and SDP down to private addresses
    (`team-webrtc.ts:618`, `:638`, `:651`, `:659`).

---

## Q3. Security: authentication beyond DTLS, fingerprint binding, identity

**Summary:** there is no application-layer encryption. Payloads are plaintext JSON or binary inside
DTLS/SCTP. **There is, however, a real end-to-end authentication handshake that binds both DTLS
fingerprints**, so a malicious Signal server or TURN relay cannot man-in-the-middle the channel.

### Keys and trust anchors
- **Client key.** Each connect generates a fresh Ed25519 keypair (`client-transport.ts:946-952`). The
  public key goes to the account service in `issueTicket(sessionId, clientPublicKey)` (`:967`). The
  returned **ticket is an ES256 JWT** with audience `openbot-remote`. It carries `hostId`, `sessionId`,
  `userId`, `membershipId`, `role`, `authEpoch`, `sessionExpiresAt` and `clientPublicKey` (`ticket.ts:3`,
  `:11-27`).
- **Host key.**
  - It is the host's long-term device Ed25519 key in `TeamStore`. `signRemoteAuthentication(transcript)`
    is `sign(null, transcript, state.privateKey)` (`team-store.ts:423-426`).
  - The client **pins** it per hostId with `pinHostKey` (`client-transport.ts:280-282`). It refuses to
    connect without one: error `host_key_missing` (`:927-930`).
  - The pinned key comes from the account service's host list (`devicePublicKey`), **trust on first use**:
    `resolveHostKey` keeps a stored key, refuses an advertised key whose fingerprint differs from the
    pinned one, and otherwise accepts the advertised key (`remote-server-host-directory.ts:228-233`).
  - A key change is followed only through `keyHandovers` signed by the trusted key (`:91-113`). Pins are
    loaded at startup from the local store (`remote-server-manager.ts:608-610`) and updated on host-list
    reconciliation (`:2719-2731`).
- **Host trust in tickets.** The host verifies the ticket JWT against JWKS fetched from
  `<apiUrl>/.well-known/jwks.json`, with `algorithms: ["ES256"]` and the audience checked. The JWKS is
  cached for the process lifetime and refetched only on `ERR_JWKS_NO_MATCHING_KEY`
  (`central-auth-manager.ts:1122-1156`, `:1201-1240`).

### DTLS fingerprints
- The renderer reads `a=fingerprint:sha-256` from the local and remote SDP and uppercases it
  (`team-webrtc.ts:802-813`). When all four channels are open, it posts `peer-connected` with
  `localFingerprint` and `remoteFingerprint` (`:738-751`).
- The fingerprints are **not checked against any external registry**, neither the account service nor the
  invitation. They are bound cryptographically through the handshake transcript described next.
- A separate check: on an answer during renegotiation, a changed remote fingerprint drops the connection
  (`team-webrtc.ts:630-635`).

### Handshake (all frames on `"rpc"`)
Transcript format (`webrtc-framing.ts:267-288`):
`JSON.stringify(["openbot-team-v2-auth", hostId, sessionId, ticket, clientPublicKey, clientNonce,
hostNonce ?? "", clientFingerprint, hostFingerprint])`

1. **Client sends `auth-init`** `{ticket, clientPublicKey, clientNonce (32 random bytes), signature}`.
   The signature is Ed25519 over the transcript with `hostNonce=""`, `clientFingerprint = local` and
   `hostFingerprint = remote` (`client-transport.ts:1240-1281`). It is sent only once `peer-connected`
   delivers a binding. With no binding, it fails (`:1252-1255`).
2. **Host checks `auth-init`** (`host-peer.ts:466-535`):
   - A pending Signal `incoming` connection and a DTLS binding must exist (`:474-476`).
   - `verifyClientTicket(frame.ticket)` checks the JWT (`:477`).
   - **The claims must equal the Signal `peer-ready` data:** hostId, sessionId, userId, membershipId,
     role, sessionExpiresAt, and `claims.clientPublicKey === frame.clientPublicKey` (`:481-493`).
   - The client's signature is verified over the transcript with the host's view: `clientFingerprint =
     binding.remote` and `hostFingerprint = binding.local` (`:494-509`). This is proof of possession of
     the ticket-bound key, bound to both DTLS certificates.
   - The host replies **`auth-ready`** `{clientNonce, hostNonce, signature}`. The signature uses the host
     device key over the transcript, now including `hostNonce` (`:510-534`).
3. **Client checks `auth-ready`** (`client-transport.ts:1356-1415`): the `clientNonce` must match, and it
   verifies the host signature with the **pinned** host public key over its own transcript (`:1372-1393`).
   It then sends **`auth-complete` {clientNonce, hostNonce}** (`:1396-1407`).
4. **Host on `auth-complete`** (`host-peer.ts:537-577`): it matches the nonces, then calls
   `#openIncomingSession`. That calls `store.openRemoteSession(...)`, which mints a **local 32-byte random
   `sessionToken`**. The token is kept only as a hash in memory, together with the member and role from
   the ticket claims (`host-peer.ts:367-388`, `team-store.ts:975-1006`).
   - The host records `sessionBinding = fingerprints` and marks the peer authenticated.
   - It replies **`auth-confirmed`** (`:564-575`).
5. **Client on `auth-confirmed`**: the nonces must match (`client-transport.ts:1417-1434`). Only then is
   it connected: `#finishConnected` sends the initial `event-ack` (`:1283-1299`).

### Why this defeats a Signal or TURN man-in-the-middle
A man-in-the-middle that rewrites the SDP fingerprints produces different fingerprints in the client's
and the host's transcripts. The client's signature then fails at the host. The host's signature fails at
the client, which checks it against the pinned key.

### Residual trust
- The account service can impersonate a host **only on first contact**, before the key is pinned. It is
  the full authority on *client* identity and role, because it signs the tickets.
- The Signal service supplies the `peer-ready` claims, but the host cross-checks them against the
  signed ticket.

### Gating before authentication
- **Host side** (`host-peer.ts:403-444`): `auth-init` and `auth-complete` are always accepted for
  processing. Any other frame on any channel triggers `#failProtocol` (close the session and disconnect
  the peer) unless `#localSessionToken` is set (`:426-428`). Binary on rpc or events also fails (`:436-438`).
- **Client side**: any data before `connected` that is not `auth-ready` or `auth-confirmed` fails the
  protocol (`client-transport.ts:1334-1338`).
- **File transport**: it ignores frames until `setPeerAuthenticated(peer, true)`
  (`file-transfer.ts:130-134`, `:469`). The host instance also requires `localSessionToken !== null`
  (`host-peer.ts:260-265`).
- **Reconnect fast path**: if a new data-channel connection arrives for the same session with *identical*
  DTLS fingerprints, the host re-authenticates without a new handshake (`host-peer.ts:345-365`). Otherwise
  it closes the local session.
- **Authentication failures** are logged as "did not pass the … sign-in step" and the peer is disconnected
  (`host-peer.ts:1075-1096`).

### Comment and string claims versus code
- The error strings "end-to-end authentication" (`client-transport.ts:1336`, `:1411`) are backed by the
  code above. The handshake is a real mutual signature exchange with channel binding.
- "End-to-end" means authentication, not content encryption beyond DTLS. The code has no extra cipher.

---

## Q4. Forwarding to the local Team API server on loopback

- The Team API server binds `127.0.0.1` on an ephemeral or preferred port (`team-api-server.ts:780`,
  `listen-loopback.ts:17-28`). The gateway receives that port as `localApiPort` and passes it into each
  `TeamWebRtcHostPeer` (`host-gateway.ts:150-168`, `:353-367`; `host-service.ts:1127`, `:1220`).
- `#dispatchHttpEffect` (`host-peer.ts:696-745`):
  - It requires `localApiPort` and `localSessionToken`, otherwise it returns 401 `remote_session_missing`
    (`:700-703`).
  - It builds `new URL(input.path, "http://127.0.0.1:<port>")` and **rejects the request unless the
    origin is unchanged and `pathname.startsWith("/v1/")`** (`:704-709`). Methods are limited to
    GET, POST, PUT, PATCH and DELETE, and the path is at most 2048 characters (`:1282-1311`).
  - Clients below protocol 4 get 426 `client_update_required` (`:722-726`).
- `#forwardHttpEffect` (`host-peer.ts:747-903`):
  - It issues a plain Node `fetch(url, {method, headers, body})` (`:806-821`; the transport is defined at
    `:250-254`) with these headers:
    - `Authorization: Bearer <localSessionToken>`. This is the per-session random token minted at
      `auth-complete`. It is **not** the account ticket.
    - `Content-Type`
    - `OpenBot-Protocol-Version`
    - `OpenBot-App-Version`
    - `OpenBot-Capabilities`, plus an optional absent-capabilities header
    - `X-OpenBot-WebRTC-Session: <sessionId>`
  - The request body is the decoded JSON re-stringified for the local wire format, or, for an upload, a
    Blob from `openAsBlob(uploaded.path)` (`:772-801`).
  - Responses: JSON is re-encoded per protocol and capability (`:855-896`). A non-JSON or
    `Content-Disposition: attachment` response is streamed back as a file transfer and returned as
    `{status, body:null, file:{transferId, name, mimeType, size}}` (`:842-854`). A non-2xx response becomes
    a GatewayError carrying the server's `code` and `error` (`:832-841`).
- Authorization of *which* `/v1/` routes a member may call is left to the Team API server, using the
  role stored with the local session token. The host-peer only enforces the `/v1/` prefix.
  `X-OpenBot-WebRTC-Session` is also used for rate-limit keying (`team-api-server.ts:1684-1690`).
- Events use `ws://127.0.0.1:<port>/v1/events` with the token in the subprotocol (`host-peer.ts:905-911`).
- Desktop streams use local WebSockets with only the session header (see Q1).

---

## Q5. Host↔host agent communication through this transport

**Absent in these files.** The transport is strictly client app → one host:
- The client side calls Team API endpoints on a host. Callers are the UI and IPC handlers through
  `remote-server-manager.ts`. I listed the importers with grep; they are `ipc/*-handlers.ts`,
  `remote-instance-transfer-chunks.ts`, `browser-view-client.ts` and others.
- The host side only forwards to **its own** loopback Team API (`host-peer.ts:704-709`). It never opens an
  outbound client connection.
- The gateway drops an `incoming` event whose peerId is the host's own id (`host-gateway.ts:354`).
- No code in the five files relays one host's agent traffic to another host, or exposes the client
  transport to agents.
- The one cross-machine "agent uses another machine" path is **browser egress**: the host's agent
  browser can tunnel TCP out through a *connected member's* client (`browser-egress-host.ts:55-66`).
  That is network egress, not agent-to-agent messaging.
- Instance-transfer (`remote-instance-transfer-chunks.ts`) exists but I did not read it. It appears to be
  a UI-driven client→host flow.

---

## Q6. File transfer (`team-webrtc-file-transfer.ts`)

- **What moves:**
  1. Binary request bodies, client→host, such as uploads. The request carries `bodyTransferId`
     (`client-transport.ts:532-539`; host side `host-peer.ts:739-742`).
  2. Non-JSON or attachment responses, host→client, streamed from the local `fetch` body
     (`host-peer.ts:842-854`).
- **Limits.** 100 MiB per file and 250 MiB in flight per peer (`webrtc-framing.ts:10-11`). They are
  enforced on send (`file-transfer.ts:234-257`) and on receive (`:632-637`, waiting opens `:515-536`).
- **Spooling.** Outgoing streams are written first to a `<uuid>.outgoing` file (mode 0600, flags `wx`) in
  a 0700 directory while SHA-256 is computed, then sent (`:166-232`).
  - The host uses a per-peer directory: `transferDirectory/sha256(peerId)` (`host-peer.ts:262`).
  - Incoming data goes to `<transferId>.part` with mode 0600 (`:639-667`). transferId is restricted to
    `[A-Za-z0-9:_-]{1,128}` (`webrtc-framing.ts:345-347`), so the path cannot traverse directories.
- **Protocol:**
  - The sender sends `file-open {transferId, name (basename, ≤255), size, mimeType, sha256}`
    (`:772-782`) and waits for a `file-ack`.
  - It then sends 60 KiB binary chunks starting at the acknowledged offset (`:25`, `:792-802`), followed
    by `file-complete` (`:803-807`), and waits until `acknowledged === size` (`:808-816`).
  - The receiver writes chunks at their offset. It accepts overlapping resends and rejects gaps or
    overruns with "offset is invalid" (`:597-613`). It acks every 1 MiB and at the end (`:26`, `:617-629`).
- **Resume.** If the peer disconnects, the sender re-sends `file-open`. The receiver answers with
  `file-ack receivedThrough = existing.received` if size and sha256 are unchanged (`:499-514`), and the
  sender resumes from there. The resume window is 10 min without progress (`:27`, `:758-829`).
  Incomplete or unclaimed files expire after the same window (`:879-919`).
- **Integrity.**
  - On `file-complete` the receiver requires `received === size`, closes the file and recomputes
    `sha256File(path)`. On a mismatch it sends `file-cancel` and fails ("The WebRTC file hash is
    invalid.") (`:546-567`).
  - The sha256 must be 64 lowercase hex characters (`webrtc-framing.ts:219-220`).
  - There is no per-chunk MAC beyond DTLS. The whole-file hash comes from the sender, so it protects
    against corruption and truncation, not against a malicious peer.
- **Ordering and safety.** All frames are processed serially, through a semaphore plus a chained
  deferred (`:110-111`, `:464-488`). Any undecodable frame without a transferId disconnects the peer
  (`:731-741`).
- **Consumption.** `useReceived` gives the file to the caller and deletes it afterwards (`:394-413`).
  `consume` reads it into memory on the client (`:378-392`).
