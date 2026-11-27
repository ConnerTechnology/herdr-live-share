# E: Account service (auth-api Worker + D1) and Signal (remote/api)

Reader: E. Source: `openbot` clone (untrusted, read only). Nothing was run or built.
Paths below are relative to the repo root. `rcp` = `apps/auth-api/src/server/remote-control-plane.ts`,
`sig` = `remote/api/src/signal-service.ts`, `d1repo` = `apps/auth-api/src/server/d1-auth-repository.ts`.
"Code" means TypeScript/SQL I read; "Docs" means Markdown claims. They are kept apart.

---

## TL;DR

- **Three parts**: (1) the account service, a Cloudflare Worker on D1 at `api.openbot.run`; (2) Signal, a
  Bun WebSocket service at `signal.openbot.run`, plus coturn at `turn.openbot.run`; (3) the host, which is
  the user's own machine running the OpenBot app. Chats, files and commands go host to client over WebRTC
  data channels (Docs: remote/README.md:3-7, 16).
- **The account service is the only membership authority.** D1 holds users, hosts, memberships (owner,
  admin, member), invites, logical sessions, and outboxes. It signs ES256 tickets. The host trusts any
  ticket that verifies against `/.well-known/jwks.json` of its configured `apiUrl` (Code:
  src/main/central-auth-manager.ts:1122-1240).
- **The account service can impersonate any member to any host.** Its private JWK can mint a ticket for
  any `userId`/`role` with a `clientPublicKey` that it controls. The host checks only the signature, the
  claims against Signal's `peer-ready`, and proof of possession of that key (Code:
  src/main/team-webrtc-host-peer.ts:466-509). A second path exists in code: the `SERVER_CLAIMS_ADMIN_TOKEN`
  mints a persistent account session for **any email**, existing account or not (Code:
  d1repo:1038-1126; Docs: docs/self-hosted-server.md:116 says so outright).
- **The cloud cannot read the WebRTC content**, and it cannot impersonate a host once the client has pinned
  that host's key. Clients pin the host's Ed25519 device key on first use (the cloud supplies it at first
  use), and they accept a key change only through a chain of handovers that the old key signed (Code:
  packages/team-client/src/remote-directory.ts:567-600, rcp:819-912). The auth transcript binds both DTLS
  fingerprints, so Signal cannot MITM the SDP either (Code: team-webrtc-client-transport.ts:1372-1392).
- **Signal stores nothing durable.** It verifies tickets locally through JWKS, relays SDP/ICE, issues HS256
  resume tokens and coturn REST credentials. It also relays Slack, Discord, Telegram, GitHub, generic
  webhooks and Apple Watch traffic. Signal reads Discord message text and Discord files in transit, and
  holds the shared Discord and Telegram bot tokens.
- **Self-hosting the whole stack is supported** (Docs: docs/self-hosting.md): your own Worker on your own
  Cloudflare account, D1 and SMTP, plus your own Signal and TURN. A self-hosted Signal requires a
  self-hosted account service. Slack, GitHub, Telegram, hosted servers and the Live Activity relay stay
  off, and Cloudflare itself (Workers/D1/R2) stays a dependency.
- **There is no federation.** One account service, one D1, one ticket key. Self-hosted accounts are
  separate from openbot.run accounts. Only one cross-server feature exists: "Copy to another server"
  (instance transfer). The client moves agents between two servers that the user already administers
  (Docs: docs/architecture/servers.md:363-392).

---

## 1. Data model (Code: apps/auth-api/migrations/*.sql)

### Users and auth sessions
- `users(id, identity_key, email UNIQUE, name, avatar_url, ...)`: 0001_google_auth.sql:1-9. Google OAuth
  was removed, and `google_subject` was renamed to `identity_key` (0003). Later columns:
  `analytics_enabled` (0043) and `chat_color` (0049).
- `auth_sessions(id, user_id, token_hash UNIQUE, expires_at, created_at, last_used_at, revoked_at)`:
  0001:24-32. Only the SHA-256 of the token is stored (README: apps/auth-api/README.md:5-7).
  **Sessions never expire.** `PERSISTENT_SESSION_EXPIRES_AT = 8_640_000_000_000_000`
  (src/server/session-policy.ts:1), and 0017:4-7 moved existing sessions to that value.
- Sign-in: email one-time codes (`email_login_challenges`, 0002/0015). The 8-character code is stored
  hashed, with 5 attempts and a 10-minute TTL (auth-service.ts:37-40, 177-211). Company OIDC sign-in exists
  per host (0047). `mobile_auth_sessions` (0014, 0033) marks phone sessions: `device_id`, `name`,
  `platform`, `connect_link_host_id`.
- Rate limits: `auth_rate_limits(key_hash, window_start, attempts)`, 0002:18-23.

### Hosts ("servers")
- Legacy: `team_tunnels` (0005), with `CREATE UNIQUE INDEX team_tunnels_user_id ON team_tunnels(user_id)`
  (0005:13). `0006_one_team_server_per_user.sql` drops and re-creates the same unique index (0006:1-2).
  0008 rebuilds the table and keeps the unique index (0008:40). That limit was one Cloudflare Tunnel per
  user, and it is now retired: `/v1/team-tunnels/provision` answers 426 (routes/v1/team-tunnels/provision.ts).
- Current: `remote_hosts(host_id PK, owner_user_id, name, logo_key, device_public_key, machine_token_hash,
  auth_epoch >= 1, ...)` (0012:1-11). `owner_user_id` has a **non-unique** index (0012:16). Later columns:
  `signal_home` (0044) and `appearance_shape/color` (0056). 0012:62-98 migrated each tunnel to a host plus an
  owner membership.
- **A user can own many hosts.** Nothing in remote_hosts limits owners. Other limits: hosted servers allow
  50 per account (Docs: docs/hosted-servers.md:24-26), and the desktop allows 5 extra "servers on this
  computer" (Docs: docs/architecture/servers.md:2131-2134).
- `machine_token_hash` is the host's bearer credential to the Worker. `registerHost` makes a random token
  and stores its SHA-256 (rcp:749-794). `authenticateHost` compares the hashes in constant time
  (rcp:3314-3335).
- `device_public_key` is the host's Ed25519 identity key, which clients pin. Key rotation goes through
  `remote_host_key_handovers` (0039). Each row holds `previous_public_key`, `public_key`, and two
  signatures: one by the old key and one by the new key (rcp:819-912 verifies both with Ed25519,
  rcp:4081-4095).
- `auth_epoch` goes up when access is revoked or the host is re-registered. Signal drops peers whose epoch
  is lower (rcp:1664-1672, 1906-1909; sig:685-701).

### Memberships and roles
- `remote_memberships(membership_id PK, host_id, user_id, role IN ('owner','admin','member'),
  status IN ('active','revoked'), ..., UNIQUE(host_id, user_id))` (0012:18-27). The index on
  `(user_id, status)` (0012:29-30) is there because **a user can be a member of many hosts**. `listHosts`
  returns every active membership (rcp:914-985). Later columns: `removed_at` (0036) and `hidden_at` (0071).
- Tickets have a 4th role, `host`, used only for the host's own Signal socket
  (packages/contracts/src/signal-protocol/ticket.ts:7). A host ticket uses
  `sessionId = "host-<id>"` and `membershipId = "<id>:host"` (rcp:2657-2667, 2321-2329).
- Finer per-agent roles (`none/view/chat/manage`) live **on the host** in
  `openbot-agent-access-v1.json`, not in D1 (Docs: docs/architecture/servers.md:1652-1691). The same is
  true of the audit log (servers.md:1708-1747).
- The member limit gates joins. Precedence: a `server_member_limits` row (0040, 1-100), then the hosted
  plan, then the owner's license users, then the default of 3 (rcp:3524-3547; Docs:
  docs/hosted-servers.md:334-384). The seat check is SQL inside the join insert
  (`MEMBER_SEAT_AVAILABLE_SQL`, `LICENSE_SEAT_AVAILABLE_SQL`, rcp:150-159). Billing gates membership
  **only** through this limit.

### Invitations
- `remote_invites(invite_id, host_id, token_hash UNIQUE, email NULLable, role IN ('admin','member'),
  created_by_user_id, expires_at, used_at, revoked_at)` (0012:32-43). `max_uses` (DEFAULT 1) and
  `use_count` came in 0020_permanent_invites.sql:1-2. `max_uses IS NULL` means a permanent multi-use link.
- Only an owner or admin creates invites (rcp:1002). A one-shot invite lives 300 s to 30 days, 7 days by
  default (rcp:1008-1011). A permanent invite takes no email and no TTL (rcp:1005-1006). Limits per host:
  50 outstanding and 5 permanent (rcp:89-90).
- An invite can be email-bound. Acceptance then needs a matching account email (rcp:1350-1356).
- Revocation: `INVITE_STILL_GRANTED_SQL` (rcp:180-191) voids an invite when its creator is no longer an
  active owner or admin. It also voids an invite for a user whose membership was revoked after the invite
  was created. 0037 back-filled this rule.
- The invite link is `openbot://join` or `https://openbot.run/join` (Docs: team-api.md:194-196). The link
  carries `serverId` and `token` (team-invite-email.ts:41-42, 77-83). `previewInvite` returns the host's
  `device_public_key` and key handovers (rcp:1253-1288). **This is where the first-use key pin comes from,
  and the cloud supplies it.**

### Logical remote sessions and tickets
- `remote_sessions(session_id, host_id, user_id, membership_id, started_at, ended_at, expires_at,
  auth_session_hash)` (0012:48-56, 0018). There is one active session per (host, user, device auth
  session) (0018:4-9).
- Triggers: revoking an `auth_sessions` row ends its `remote_sessions` and queues
  `remote-session-ended` into `remote_auth_events` (0018:12-25). It also revokes `watch_sessions` (0077:20-26).
- `remote_auth_events(event_id, payload, attempts, next_attempt_at, pending_targets)` (0013, 0044) is a
  durable outbox. Its events go to Signal at `/internal/auth-events` over HMAC (rcp:4005-4076).
- `team_auth_tickets(ticket_hash, user_id, server_id, expires_at, consumed_at, mobile_host_id,
  mobile_host_fingerprint)` (0004, 0017). This is the legacy HTTPS-host identity ticket and the Mobile
  Connect one-shot code. `issueTeamAuthTicket` does **not** check membership. It only authenticates the
  caller and stores (user, serverId) for 2 minutes (auth-service.ts:607-634). Redeeming returns only the
  user profile (d1repo:606-636). On legacy HTTP(S) hosts the host keeps its own member list (Docs:
  team-api.md:116-118).
- `mobile_connect_links` (0032) is a reusable phone link that a desktop session makes. It is bound to a
  host fingerprint, which is the SHA-256 of `device_public_key` (d1repo:703-859, 1209-1213).
- `server_claims(claim_hash, user_id, expires_at, redeemed_at, session_id)` (0034_server_claims.sql)
  holds a one-time claim that redeems into a full auth session (see §2).
- `remote_owner_transfers` (0060): a 7-day transfer of ownership to an email (rcp:1715-1965).
- `watch_sessions` (0077) is the Apple Watch session. It holds a P-256 `public_key`, belongs to a parent
  auth session, and lives 30 days (watch-sessions.ts).

### Messaging routes (D1 maps a platform id to a host)
- `slack_workspace_routes(team_id PK, host_id, account_id, app_id, bot_user_id, connected_at)` (0025).
- `discord_guild_routes(guild_id PK, host_id, account_id, connected_at)` (0026).
- `webhook_routes(route_id, host_id, account_id, connected_at, revoked_at)` (0027).
- `telegram_chat_routes(bot_id, chat_id, host_id, account_id, linked_at)` and `telegram_link_codes` (0029).
- Company sign-in tables (0047): `company_sign_in_connections` holds per-host OIDC settings with the
  client secret encrypted under `COMPANY_SIGN_IN_SECRET`. The others are domains, identities, users
  (managed by SCIM), flows, and codes.

### Ticket shapes (Code)
- Remote ticket (ES256, `aud: openbot-remote`, 180 s TTL, rcp:84, 383-423) has these claims:
  `sessionId, hostId, userId, membershipId, role, authEpoch, protocolMinimum/Maximum, sessionExpiresAt,
  clientPublicKey?, signal?` (contracts ticket.ts:11-27). The **client** ticket carries `clientPublicKey`
  (rcp:2363-2413). The **host** ticket has no key and an infinite `sessionExpiresAt` (rcp:2623-2671).
- The route tickets for Slack, Discord, Telegram, webhooks and GitHub are ES256 JWTs with
  `hid` plus a route list (rcp:448-541, 2673-2705). The watch ticket uses `aud: openbot-watch` (rcp:425-446).
- Slack, Telegram and Discord route tickets use separate route keys. Their public keys share the same
  JWKS (rcp:614-629), and audiences keep the ticket types apart.

---

## 2. What the account service and Signal store and see

### Account service (Code)
- **Stores:** emails, names, avatar URLs (R2), chat color, analytics opt-in, hashed session tokens with
  last-used times, mobile device names and platforms, hosts (name, logo, appearance, device public key,
  machine token hash), every membership and role, invites (with optional invitee email), remote sessions
  (who connected to which host, and when), messaging route ids (Slack team, Discord guild, Telegram chat,
  webhook route ids), watch sessions, and the encrypted company OIDC client secrets. It also stores
  billing, credits and analytics, which are out of scope.
- **Sees transiently:** the Slack OAuth exchange. The Worker receives the workspace **bot token** in
  plaintext from `oauth.v2.access`, then seals it to the host's P-256 key and returns it in the URL hash.
  It does not store the token (slack-app.ts:129-213). Discord's OAuth access token is revoked at once
  (discord-app.ts:141, 190-199), and the Discord bot token lives in Signal, not the Worker.
- The GitHub route lets the Worker use the host's GitHub token to list repositories (Docs:
  remote/README.md:68-72; routes/v2/remote/hosts/$hostId/github-route.ts).
- Push relays carry only sealed content. APNs and FCM payloads hold the host-sealed blob with the fixed
  title "OpenBot" (live-activity-relay.ts:147-170, android-notification-relay.ts:378-386). The Worker
  knows which host sends a push and to which device token.
- **Does not see:** chats, files, commands, or agent output. Those travel host to client over WebRTC
  (Docs: AGENTS.md:3-4: "It never holds chats, files or commands"; README remote control plane:
  apps/auth-api/README.md:446-449).
- Signal homes: the Worker places hosts by Cloudflare `request.cf` latitude, longitude and country
  (routes/v2/remote/hosts/$hostId/ticket.ts:48-56; rcp:3576-3602). So the Worker sees each host's
  approximate location.

### Can the account service impersonate a member to a host? **Yes.**
1. **Ticket minting.** The host verifies a client's `auth-init` this way (src/main/team-webrtc-host-peer.ts:477-509):
   - it checks the ticket against JWKS fetched from `<apiUrl>/.well-known/jwks.json` and caches it
     forever (central-auth-manager.ts:1201-1240);
   - it checks that the claims equal Signal's `peer-ready` values;
   - it checks that the client signed a transcript with the ticket's `clientPublicKey`.
   Whoever holds `REMOTE_TICKET_PRIVATE_JWK` can mint `{userId: victim, role: owner, clientPublicKey:
   attacker}` and pass every check. The self-hosting doc says so: "Keep the private JWK secret. With it,
   a person can open each host that uses your service." (docs/self-hosting.md:50, also :7-9).
2. **Admin session minting.** `POST /v2/admin/server-claims {email}` with `SERVER_CLAIMS_ADMIN_TOKEN`
   (request-auth.ts:512-514; routes/v2/admin/server-claims.ts) runs `createServerClaim`. That inserts the
   user if the email is new and creates a 15-minute claim (d1repo:1038-1063). `POST
   /v2/server-claims/redeem` then gives a **persistent auth session for that user** (d1repo:1081-1126).
   Docs: "Anyone with the admin token can get a session for any email, also of an existing account."
   (docs/self-hosted-server.md:116). The owner can see and revoke it in their account sessions (:119-120).
3. **Store-review bypass.** `STORE_REVIEW_EMAIL` and `STORE_REVIEW_CODE` give one email a fixed sign-in
   code (auth-service.ts:196-199, 974-983; README:340-347).
4. **Owner-transfer session minting.** After an accepted transfer, the host's machine token can obtain a
   fresh persistent auth session of the new owner (`issueTransferredOwnerSession`, rcp:1967-2062).
5. **Membership is whatever D1 says.** The host syncs its member list from the account directory and
   closes sessions that D1 no longer allows (Docs: team-api.md:113-115; servers.md:2108-2113). Anyone who
   can write D1 can add members.

### Can it impersonate a host to clients? Only on first contact.
- A host ticket (`role: host`) needs the machine token, and the Worker holds only its hash. But the
  ticket key can sign one regardless, which would admit the bearer to Signal as the host.
- Clients verify the host by an Ed25519 signature over a transcript that includes both DTLS fingerprints,
  checked against the pinned `hostPublicKey` (team-webrtc-client-transport.ts:1372-1392). The pin is TOFU
  from the invite preview or directory, and later changes follow signed handovers
  (remote-directory.ts:261, 538, 567-600). So a malicious cloud can MITM only a client that has never seen
  the host. That includes a new joiner, because `previewInvite` returns `device_public_key` from D1
  (rcp:1275-1285). Mobile Connect QR codes carry the fingerprint out of band (d1repo:1209-1213).

### Signal (Code + Docs)
- **No database** (Docs: remote/README.md:18-21). All state is in process memory: peers, routes, resume
  token cache, revoked epochs and sessions, rate windows, and the sealed ingress queue (sig:347-395).
- **Sees:** ticket claims (user id, host id, membership id, role, session id), IP addresses, SDP and ICE
  (it relays them, sig:584-591), per-user connection counts, and which client talks to which host.
- It logs no tokens, bodies or SDP (Docs: remote/api/AGENTS.md:48-52). That is a policy; I did not
  verify every log call.
- **Can Signal read conversations?** Not WebRTC traffic. The DTLS fingerprints sit inside a transcript
  that both ends sign, so an SDP swap fails authentication.
- **Can Signal impersonate a member to a host?** Not by itself. It holds `REMOTE_SESSION_SECRET` and can
  mint HS256 resume tokens (tokens.ts:435-464), which admit any claims at the Signal layer. But the host
  verifies the ES256 ticket from the account service inside the data channel, and an HS256 resume token
  fails that check. Signal can deny service, misroute, and forge `peer-ready` metadata, which the host
  then cross-checks against the ticket (team-webrtc-host-peer.ts:481-492).
- Signal **does** see these bodies in transit: Slack event bodies (it reads only ids and types and passes
  the bytes on), Telegram updates, GitHub webhooks, generic webhooks, and **Discord message text and
  files**. Signal builds the Discord deliveries itself from the Gateway, so it holds the text
  (discord-events.ts in the read, lines 375-409; README:118-119: "So the answers and files of Discord
  conversations pass through Signal").
- Telegram bot API results pass through Signal in reduced form, and files stream through it
  (app.ts:445-482).
- The queue for a sleeping host seals each event to the host's P-256 queue key
  (packages/contracts/src/signal-protocol/ingress-queue.ts:32-36). Signal seals the plaintext itself, so
  it saw the plaintext before it sealed it.
- Watch relay: the request and reply are sealed between the watch and the host (sig:954-1030; AGENTS.md:33-37).

---

## 3. Signal service details (Code: remote/api/src)

### WebSocket `/v1/signal` (app.ts:483-509)
- Limits: 64 KiB frames, 120 s idle timeout, pings. Each socket must send `hello` within 30 s
  (sig:326, 481-488).
- `hello {peer: host|client|ingress, token, multiplex?, waitForHost?, slackRoute?, discordRoute?,
  webhookRoute?, githubRoute?, telegramRoute?, queueKey?, webhookFormats?, watchRelay?}`
  (protocol.ts:149-165).
- Authentication (sig:1408-1729):
  - Signal first tries the token as an ES256 ticket through local JWKS (tokens.ts:258-269). A ticket is
    single use (`usedTicketIds` replay cache, sig:1512-1513).
  - If that fails, it tries an HS256 resume token (tokens.ts:408-433). Resume tokens live 10 min and are
    cached by jti. On a miss, such as after a restart, Signal makes one HMAC-signed call to
    `/v2/remote/resume/validate` (index.ts:117-139). The Worker then checks session, membership, role,
    epoch and expiry (rcp:2313-2361). A stale token, expired up to 24 h, can be renewed after validation
    (tokens.ts:499-521).
  - For 3 minutes after a start, Signal also validates fresh tickets online (`mustValidateTicket`,
    sig:390-391, 1731-1733).
  - `peer=host|ingress` requires `role=host`. `peer=client` requires a member role (sig:1432-1435).
  - The `signal` claim pins a ticket to one Signal machine (tokens.ts:699-702). A ticket with no claim
    goes to the entry machine only (sig:1431).
- Pairing: a client's hello finds the host's current socket and creates a `connectionId`. The client gets
  `ready{connectionId, resumeToken, iceServers, reconnectGraceMs}`. The host gets `peer-ready{connectionId,
  sessionId, userId, membershipId, role, sessionExpiresAt, resumed}` (sig:1699-1727).
  - A legacy host without `multiplex` allows one client and answers `host_busy` to the rest (sig:1686-1696).
  - A host keeps at most 4 sockets (sig:1647-1660).
  - `waitForHost` parks a client until the host appears (sig:1673-1677).
- SDP and ICE: `offer`/`answer`/`ice-candidate`/`ice-restart` frames go only to the other end of a
  connection that the sender owns (sig:584-591).
- TURN: `turn-refresh` re-issues a resume token and credentials, reusing the credentials for 5 min
  (sig:522-544).
- Limits: 32 client sockets per user, 32 sockets per IP, and 600 messages/min. Hosts and ingress get a
  10x message budget (config.ts:155-157; sig:1928-1934).

### TURN credentials (tokens.ts:523-541)
- They use the coturn REST scheme: `username = "<expiry>:<sessionId>"` and
  `credential = base64(HMAC-SHA1(TURN_SHARED_SECRET, username))`. Expiry is the smaller of the session
  expiry and now + `SIGNAL_TURN_CREDENTIAL_TTL_SECONDS`.
- Signal returns `stun:` plus `turn:` UDP, TCP and `turns:` TLS URLs for one relay. It picks the relay
  per host by rendezvous hash over the relays that probe healthy (turn-relays.ts:48-61). A real allocation
  probe runs each minute (turn-probe.ts).

### Revocation path
- The Worker writes an event to its outbox and POSTs it to Signal `/internal/auth-events`. The request is
  signed with HMAC-SHA256 of `timestamp.body` under `REMOTE_AUTH_WEBHOOK_SECRET`, with a ±5 min window
  (app.ts:194-222; tokens.ts:676-693).
- Event types (auth-events.ts):
  - `remote-auth-changed{hostId, authEpoch}` drops lower-epoch peers (sig:685-701);
  - `remote-session-ended` closes one session (sig:804-822);
  - `account-servers-changed` and `account-profile-changed` push a nudge to the user's sockets
    (sig:671-683);
  - `*-route-revoked` events remove a messaging route.
- Retries back off exponentially up to 1 h. The cron also retries (rcp:4064-4072; worker-handler.ts:126).
- Signal calls the Worker with the same HMAC for these paths: resume/validate, the slack, discord,
  webhook and telegram route validate paths, `telegram-route/link`, `discord-route/removed`,
  `discord-route/reconcile`, `route-wake` and `host-wake` (index.ts:85-296; each Worker route calls
  `verifyRemoteServiceRequest`, request-auth.ts:579-588).

### Messaging relays
- **Ingress socket.** The host opens a separate `peer=ingress` socket that carries route tickets from
  the Worker. Signal checks the `hid` against the ticket host (tokens.ts:294-402). For the first route TTL
  after a start, it also validates online (sig:1453-1506).
- **Slack** (app.ts:223-272). Signal verifies the `v0` HMAC per app id; one secret per app id, comma
  separated (config.ts:63-75; tokens.ts:544-557). It reads only the app id, team id, challenge, and event
  type, channel type, thread, bot id and subtype (app.ts:91-108, 549-579). It forwards the raw body to the
  ingress socket and waits up to 2.5 s for the host's reply, which becomes the HTTP response (sig:842-883,
  1065-1116; limits sig:314-319). If no host answers, the result is a 503, so Slack retries, or a 200 when
  the event goes into the queue for a sleeping hosted server. The host calls Slack with its **own**
  sealed bot token.
- **Discord.** Signal holds **one shared bot token** and Gateway connection (discord-gateway.ts:81-258),
  with the MessageContent intent unless it is off (config.ts:119-130). It passes on only guild messages
  that mention the bot. It strips the mention and sends author, roles, text, the replied-to quote and CDN
  attachment URLs (discord-events.ts in read: `normalizeGuildMessage`). A DM gets a fixed reply once an
  hour (`#directMessage`). The host acts through `POST /v1/discord/api` with a `discord-session` bearer
  that Signal issued over the ingress socket. Signal hashes and stores it (sig:1636-1641, 770-776;
  app.ts:384-408). Signal checks that the guild is routed to that socket and the channel is in that guild
  (discord-api.ts:351-367). Mentions are limited to users (discord-api.ts:307-309). Uploads go up to 10 MB.
- **Telegram.** Signal holds the bot tokens. Each bot's webhook secret is
  `base64url(HMAC(TELEGRAM_WEBHOOK_SECRET, "telegram-webhook:<botId>"))` (tokens.ts:571-579). Signal reads
  only the chat id, callback id, `/start <code>` and addressing flags (app.ts:110-136, 754-810). A link
  code makes Signal call `/v2/remote/telegram-route/link` (rcp:3136-3215). The host calls the Bot API via
  `telegram-call` with a strict allowlist of 9 methods, strict schemas, only for chats routed to that
  socket (protocol.ts:89-147; sig:1276-1325). Files use 2-minute HMAC tokens, and an upload token works
  once (tokens.ts:611-674).
- **GitHub.** Signal verifies `X-Hub-Signature-256` and fans the event out to every host that routes the
  repository. Bodies go up to 512 KiB (app.ts:311-353; sig:926-948).
- **Generic webhooks.** `POST /v1/webhooks/events/:routeId`. Signal checks the headers are well formed
  but **not** the signature: "only the host can check the signature" (Docs: README:166-167). It forwards
  timestamp, delivery id, signature and body to the host (sig:896-920; app.ts:273-310).
- **Sleeping hosted servers** (ingress-queue.ts). Signal asks `/v2/remote/route-wake`, at most once a
  minute per route. It keeps up to 64 events, 192 KB and 10 min per host, sealed to `queueKey`. It flushes
  them as `queued-delivery` and waits for an ack.
- **Watch relay.** `POST /v1/watch/request` checks the watch ticket, the clock and a P-256 proof over the
  sealed digest. It forwards the sealed bytes to the host's ingress socket (sig:954-1030).

### Two Signal machines and failover
- An entry machine is chosen by DNS: the machine whose IP the entry name resolves to is the entry
  (entry.ts). The Worker cron moves the A record when the entry machine is unhealthy
  (signal-entry-failover.ts:52-85).

---

## 4. Self-hosting

**Docs** (docs/self-hosting.md):
- You can run **all three parts** yourself: your own Worker (Cloudflare Workers, D1, R2 on your own
  Cloudflare account), your own SMTP, your own Signal, and coturn on a Linux box (:3-32).
- You generate your own ES256 ticket key (:34-40) and deploy with `scripts/deploy-auth-api.ts` (:91-95).
  You point Signal at your JWKS and control plane (:106-113).
- You start the app with `OPENBOT_AUTH_API_URL` and `OPENBOT_REMOTE_SIGNAL_URL` (:131-169). Docker and
  the server use the same variables (docker.md:476-477; self-hosted-server.md:127 `--auth-api-url`).

Why Signal cannot be self-hosted alone (Docs: :7-10): "Our Worker does not issue a ticket for another
Signal. A ticket gives access to a host, and our key signs it, so a ticket for your Signal would also open
our Signal."

What stays dependent or off (Docs: :12-21):
- Accounts are separate from openbot.run.
- Every device must use the same service.
- Invitations are links only, with no email.
- `openbot.run/app` (the web client) does not work.
- **Hosted servers, the GitHub App, Slack, the Telegram bot, and the iPhone Live Activity relay stay off,
  because each needs secrets that only OpenBot holds.**
- Phone email sign-in always uses openbot.run, so a phone must pair by QR from a self-hosted desktop
  (:161-163).
- Still a dependency: Cloudflare as the platform (D1, R2, Workers; :25). The code also uses Cloudflare
  rate-limit bindings and `request.cf` (types.ts:10-22; ticket.ts route).
- Discord and Telegram are possible with your own bots: the Signal config reads `DISCORD_BOT_TOKEN` and
  `TELEGRAM_BOT_TOKENS`. But the guide says to leave them empty (:112-113).
- Not stated: whether the desktop still calls `api.openbot.run` for other features, such as the
  marketplace, updates or feedback. PRIVACY.md is referenced (:21) but was **not read**.

Code that confirms self-hosting:
- The JWKS can come from a URL or from inline keys (config.ts:134-137; tokens.ts:221-231).
- The control plane URL is configurable (config.ts:147).
- Signal home and entry settings are optional (config.ts:192-209).
- The host's JWKS fetch uses `this.#options.apiUrl` (central-auth-manager.ts:1230).

Legacy HTTP(S) hosts and the "HTTPS arm": an older transport where the host keeps its own member store and
the account service only vouches for identity through `team_auth_tickets` (central-auth-manager.ts:390-410,
1581-1608; team-api.md:116-118). "Servers on this computer" run as password hosts on loopback
(servers.md:2056-2063). I did not trace the HTTPS-arm host side (another reader's area).

---

## 5. Federation or linking servers or teams

- **Nothing in the data model or code links two hosts, two teams, or two account services.**
  - Memberships are always (one host, one user, one D1).
  - Tickets carry no issuer field beyond the JWKS that the host is configured with.
  - The host fetches JWKS only from its own `apiUrl` (central-auth-manager.ts:1230).
  - A host cannot trust two account services.
- Self-hosted and openbot.run accounts are explicitly separate (self-hosting.md:14-15).
- The closest feature is **"Copy to another server"** (servers.md:363-392). The user administers two
  servers, and the client exports agents from one and imports them into the other through the
  instance-transfer routes. That is a one-time copy, with no live link.
- Signal homes (`REMOTE_SIGNAL_HOMES`) are several Signal machines under **one** account service. That
  is horizontal scaling, not federation (rcp:3788-3815; apps/auth-api/README.md:451-488).
- Owner transfer (rcp:1715-1965) moves a host between accounts in the same D1.
- Agent-to-agent links are per host. `send_message` and `list_agents` run inside one host, limited by the
  member's levels (servers.md:1671-1680).

---

## Notable details for a competing design

- A self-hosted operator holds the full trust root: anyone with the ticket private key or D1 write access
  owns every host. There is no end-to-end membership proof. The host does not keep its own signed roster,
  and the owner signs nothing per invite.
- The host key is pinned on first use, and the first pin comes from the cloud. A cloud MITM of a new
  joiner is possible. Mobile Connect fixes this with an out-of-band fingerprint in the QR code.
- Resume tokens let a client reconnect without the cloud. Revocation is push-only: an outbox to Signal
  with retries (remote/README.md:23-33).
- Tickets live 180 s and are single use at Signal. Logical sessions never expire. They end only by
  revocation.
- Signal is the only party that touches Slack, Discord and Telegram traffic, and for Discord it sees the
  content.

---

## Files

### Read in full
- All 81 files in `apps/auth-api/migrations/` (0001-0078, including both 0034 and both 0071).
- `remote/api/src/`: app.ts, signal-service.ts, tokens.ts, config.ts, protocol.ts, index.ts,
  ingress-queue.ts, entry.ts, turn-relays.ts, turn-probe.ts, telegram.ts, capacity-report.ts,
  discord-gateway.ts, discord-events.ts, discord-api.ts. These are all non-test files.
- remote/README.md, remote/compose.yaml, remote/api/AGENTS.md.
- apps/auth-api/AGENTS.md, apps/auth-api/README.md.
- `apps/auth-api/src/server/`: remote-control-plane.ts (all 4173 lines), auth-service.ts,
  d1-auth-repository.ts, request-auth.ts, types.ts, worker-handler.ts, admin-token.ts, session-policy.ts,
  crypto.ts, owner-transfer.ts, team-invite-email.ts, signal-entry-failover.ts, watch-sessions.ts,
  slack-app.ts, discord-app.ts, live-activity-relay.ts, android-notification-relay.ts. Also
  src/worker-entry.ts.
- Every route file under `apps/auth-api/src/routes/v2/remote/**`, `v2/server-claims/*`,
  `v2/admin/server-claims.ts`, `v2/hosting/claims/redeem.ts`, `v2/slack/*`, `v2/discord/*`,
  `v1/team-auth/*`, `v1/team-hosts/*`, `v1/team-invitations/*`, `v1/team-tunnels/*`, `v1/mobile-auth/**`,
  `v1/auth/**`, `v1/me.ts`, and `v1/me/profile.ts`.
- `packages/contracts/src/signal-protocol/ticket.ts` and `auth-events.ts`. I read `ingress-queue.ts` in
  part, the sealing function only.
- docs/self-hosting.md and docs/self-hosted-server.md.

### Read in part (targeted)
- company-sign-in.ts: lines 429-848 only. These cover sign-in, exchange, SCIM, join and the account link.
  I mapped the rest by grep.
- docs/architecture/servers.md: lines 1-400, 1652-1850, and 2049-2345.
- docs/hosted-servers.md: lines 1-57 and 322-384.
- docs/docker.md: lines 1-102 and 468-563.
- docs/multi-tenant-hosting.md: lines 1-90.
- docs/architecture/team-api.md: lines 80-124.
- Desktop cross-checks:
  - src/main/team-webrtc-host-peer.ts:460-530;
  - src/main/team-webrtc-client-transport.ts:1355-1399;
  - src/main/central-auth-manager.ts:380-410, 1120-1240, 1575-1610;
  - packages/team-client/src/remote-directory.ts, grep for pinning only.

### Skipped
- Billing, credits, marketplace, content and sales code (they do not gate membership beyond the member
  limit, which is noted): billing-*.ts, stripe-client.ts, credit-*.ts, referrals.ts, license-service.ts
  (except where rcp calls it), custom-server-offers.ts, skill-marketplace.ts, agent-marketplace.ts,
  agent-templates.ts, marketplace-*.ts, sales-analytics-delivery.ts, account-analytics.ts,
  account-attribution.ts, account-activity.ts, page-markdown.ts, content-feed.ts, media-gateway.ts,
  near-intents-relay.ts, the email delivery files, the hosted-site-*.ts files, and the whole of src/lib.
- hosted-server-service.ts (3307 lines). It holds the hosted-server claim and session logic, which I did
  not read.
- github-installation-tokens.ts, company-sign-in-scim.ts, auth-data-retention.ts, browser-api.ts.
- remote/scripts (check.ts, protocol-e2e.ts, update.ts), the coturn config, nginx files, and
  remote/.env.example.
- Remaining contract files: watch-relay.ts, host-grant, slack-workspace-grant, discord-api contract and
  messages.ts.

### Not read
- The `.well-known/jwks.json` route body, which is trivial: it serves `publicJwks()`.
- PRIVACY.md, referenced by the self-hosting doc.
- docs/architecture/servers.md "Admin capabilities" (lines 1174-1651) and Billing (lines 1851-2048).
