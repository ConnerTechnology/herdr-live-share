# OpenBot host main process: account service, identity, secrets, watch relay, mobile connect

Read-only source study. Paths are relative to `openbot/` (repo root). `M:` = `src/main/central-auth-manager.ts`.
"Code does" means behavior visible in the source. "Comment/string claims" means text that only describes intent.

## Files read in full (top to bottom)

| File | Lines |
|---|---|
| src/main/central-auth-manager.ts | 2987 (5 chunks of 600) |
| src/main/central-auth-records.ts | 335 |
| src/main/central-auth-effects.ts | 32 |
| src/main/account-service-access.ts | 67 |
| src/main/account-service-platform.ts | 68 |
| src/main/stored-p256-key.ts | 61 |
| src/main/secret-storage-cipher.ts | 173 |
| src/main/mobile-connect-host.ts | 89 |
| src/main/watch-relay-commands.ts | 418 |
| src/main/watch-relay-key.ts | 35 |
| src/main/watch-relay-service.ts | 264 |
| src/main/watch-relay-snapshot.ts | 483 |

Imports followed and read in full, because the crypto lives there:
- packages/contracts/src/signal-protocol/watch-relay.ts (264)
- packages/contracts/src/host-grant.ts (139)
- packages/contracts/src/host-key-handover.ts (65)
- packages/contracts/src/mobile-connect.ts (144)
- packages/team-client/src/host-key-handover.ts (53)

Imports followed with short excerpts only:
- src/main/application-services.ts:579, 1744, 3335-3360 (how the watch key and the cipher are wired)
- src/main/host-service.ts:843-846, 991-1010 (host binding and registration input)
- src/main/team-store.ts:435-462 (host identity key generation)
- src/main/team-webrtc-client-transport.ts:940-990 (client key generation)
- src/main/team-webrtc-host-peer.ts:466-535 (host-side ticket and proof check)
- I also listed `apps/` and grepped for `WATCH_RELAY_PATH`. The account service source is in this repo at `apps/auth-api/`. The Signal relay is at `remote/api/src/app.ts` (it imports `WATCH_RELAY_PATH`).

Skipped: everything else, including the account-service server code (apps/auth-api), the Signal server (remote/api), signal-ingress.ts, the team-store persistence of the host private key, push-notifications.ts, and company-sign-in-browser.ts.

---

## 1. Account service

### What it is and where it lives
- The base URL is `https://api.openbot.run` in packaged builds, `http://127.0.0.1:3100` in dev, and can be overridden with `OPENBOT_AUTH_API_URL` (src/main/account-service-access.ts:45-50):
  ```ts
  process.env.OPENBOT_AUTH_API_URL,
  app.isPackaged ? "https://api.openbot.run" : "http://127.0.0.1:3100",
  ```
- `readCentralAuthApiUrl` accepts only HTTPS, or HTTP on loopback, with path `/` (M:2719-2726). Mobile Connect can use a separate origin (`OPENBOT_MOBILE_AUTH_API_URL`, account-service-access.ts:59; M:2728-2732).
- Who runs it: OpenBot's own service. Its server code ships in this repo (`apps/auth-api/src/routes/v1|v2/...`). The default domain belongs to OpenBot. The `cf-ray` header check (M:2829-2835) suggests it sits behind Cloudflare.
- Transport is Electron `net.fetch` with `credentials: "omit"` and `cache: "no-store"` (account-service-access.ts:30-43). Bearer tokens go in headers. There is no client certificate and no certificate pinning.
- TLS interception is detected, not bypassed. Certificate errors such as `SELF_SIGNED_CERT_IN_CHAIN` and `ERR_CERT_AUTHORITY_INVALID`, and HTML "filter pages" without `cf-ray`, become a `NetworkBlockedError` (M:2796-2840). Trust rests on normal Chromium/OS TLS validation.

### How a desktop signs in (all against the account service)
1. **Email one-time code.** `POST /v1/auth/email/start` with `{email}` and an `Idempotency-Key` returns `{challengeId, expiresAt, resendAt?, developmentCode?}` (M:1830-1842; records:307-327). Then `POST /v1/auth/email/verify` with `{challengeId, code, referralCode?}` returns `{sessionToken, user}` (M:1902-1910; records:329-335).
2. **Company SSO** (triggered by the error code `company_sign_in_required`, M:1861-1864). The client runs PKCE: `verifier = randomBytes(32)`, `challenge = sha256(verifier)` (M:1990-1991). It calls `POST /v1/auth/company/start` with `{email, returnTo, clientState, clientChallenge}`, which returns an `authorizationUrl`. That page opens in an in-app browser. Then `POST /v1/auth/company/exchange` with `{code, codeVerifier}` returns `{sessionToken, user}` (M:1993-2014).
3. **Claims and transfers that also produce a session:**
   - `/v2/hosting/claims/redeem` (M:2157-2179)
   - `/v2/server-claims/redeem` (M:2181-2201)
   - Owner transfer: `POST /v2/remote/hosts/:id/owner-session` with `{machineToken, previousSessionToken}` (M:2203-2229)
4. **Logout:** `POST /v1/auth/logout`. Errors are swallowed and local state is cleared (M:2283-2301; the empty `if` is at 2294-2295).
5. **Revocation:** any 401 on an authorized request is re-checked with `/v1/me`. If that also returns 401, the device signs out (M:2522-2552). At startup the stored session is confirmed with `/v1/me`, in the background unless `confirmSessionAtStart` is set (M:1752-1798).

### Tokens and keys the desktop gets from the account service
- **`sessionToken`**: an opaque bearer token for the user account (`Authorization: Bearer …`, M:2519-2521). The client never verifies it.
- **`machineToken` (per host)**: returned by `POST /v2/remote/hosts/register` (M:603-639; records:153-165). It must match `/^[A-Za-z0-9_-]{32,128}$/` when restored (M:2652). It is kept in `#teamHostTokens` (a hostId-to-token map). It goes in request bodies for every host-as-principal call (ticket, signal-homes, webhook/Slack/Discord/Telegram/GitHub routes, push, live-activity, owner-session) or in the `CREDIT_MEDIA_HOST_*` headers (`#hostHeaders`, M:683-692). All machine tokens are cleared when the account changes (M:1911-1913, 2265-2267).
- **Team auth ticket**: `POST /v1/team-auth/ticket` with `{serverId}` returns `{ticket, expiresAt}` (M:390-412). A server redeems it with `POST /v1/team-auth/redeem` with `{ticket, serverId}`, which returns the user (M:1581-1609).
- **Remote session ticket**: `POST /v2/remote/sessions/` with `{hostId}`, then `POST /v2/remote/sessions/:id/ticket` with `{clientPublicKey}` (M:1080-1120). This is a JWT (see section 2).
- **Host ticket for Signal**: `POST /v2/remote/hosts/:id/ticket` with `{machineToken, signalHome, failedSignalUrl?, preferredSignalHome?}` returns a `RemoteSessionTicket` bootstrap (M:769-792).
- **Signal homes**: `POST /v2/remote/hosts/:id/signal-homes` with `{machineToken}` returns `[{id, url}]`, where `url` must be `ws:` or `wss:` (M:193-203, 794-807).
- **Mobile Connect tickets and links** (section 4).
- **JWKS**: `GET /.well-known/jwks.json` (M:1226-1240) for verifying tickets.

### How a host registers
- `registerRemoteHost` sends `POST /v2/remote/hosts/register` (M:603-628). The body is:
  ```ts
  { hostId, name, ownerMembershipId, devicePublicKey?, automatic?,
    rotateCredential: !storedMachineToken, ...(storedMachineToken ? { machineToken } : {}) }
  ```
  `devicePublicKey` is the host's Ed25519 SPKI PEM identity key (host-service.ts:1001-1006; team-store.ts:442-446).
- **Key rotation**: `POST /v2/remote/hosts/:id/key-handover` with `{machineToken, handover}` (M:641-671). The `HostKeyHandover` carries `{hostId, issuedAt, previousPublicKey, publicKey, previousSignature, signature}` (contracts/host-key-handover.ts:4-11). Without a machine token, the client only checks whether the account service already lists `devicePublicKey === handover.publicKey` (M:648-651).
- **Signal**: the host never registers with Signal directly in these files. It asks the account service for a ticket (`/v2/remote/hosts/:id/ticket`) and a list of Signal homes. A code comment says "`false` keeps the stored home: the host peer of this build still connects to it" (M:783-784). This implies the account service records which Signal home each host uses.

### What the account service learns
From request and response shapes in these files:
- **User:** id, email, name, avatar image, analytics opt-in plus `ageMs`, and chat color (M:2303-2431; records:93-109).
- **Server/host:** hostId, name, logo bytes and version, appearance, `devicePublicKey`, key handovers, owner membership, hidden flag, member limit, hosted/storage/payment/custom plan (records:25-43; M:1502-1549, 1403-1416).
- **Members:** email, name, avatar, chat color, role, active/revoked status, createdAt (records:68-77, 281-305). The account service is the system of record for membership, roles, invites, and owner transfers (M:1256-1500).
- **Invites:** inviteId, optional email binding, role, expiry, use count (records:45-66, 203-265). The account service also sends invite emails, so it sees `{email, serverName, inviteUrl, role}` (M:1611-1632).
- **Connection metadata:** every remote session is created through the account service with `{hostId}`, so it knows which user connects to which host and when (M:1080-1092). The same applies to watch tickets (`watchSessionId`, `jti`) and Mobile Connect devices: name, platform, connectedAt, lastActiveAt, and whether they joined by code or link (records:128-151; M:532-601).
- **Company SSO:** settings, test runs, and SCIM tokens per host (M:2033-2147).
- **Integrations:** webhook, Slack, Discord, Telegram, and GitHub route tickets. **The GitHub token itself is sent to the account service:** `body: JSON.stringify({ machineToken, githubToken, repositories })` (M:824-843).
- **Push notifications and iOS Live Activity:** pushes are relayed through the account service with the machine token (M:1014-1078). These files don't show what the payloads contain (see push-notifications.ts, not read).
- **Generic relay:** `relayFetch` forwards arbitrary requests to account-service paths as either the host or the user (M:708-735). Account activity is posted as well (application-services.ts:3335-3340, `ACCOUNT_ACTIVITY_PATH`).

---

## 2. Identity and keys

### There are three key types. `stored-p256-key.ts` holds an encryption key, not a signing key.
1. **The P-256 key in stored-p256-key.ts is an ECDH (encryption) key.** It calls `createStoredHostGrantKeyPair()` (stored-p256-key.ts:40), and contracts/host-grant.ts:25-30 shows:
   ```ts
   crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"])
   ```
   The private key is exported as PKCS8 and imported back with `["deriveBits"]` only (host-grant.ts:32-36). Its only user in these files is the **watch relay key** (`loadWatchRelayKey`, watch-relay-key.ts:11-17), stored at `userData/openbot-watch-relay-key-v1.json` (application-services.ts:579, 3348). The host generates it lazily on first load (stored-p256-key.ts:39-60). Its public half is published as `watchKey` (watch-relay-service.ts:96-104; watch-relay-snapshot.ts:453). It decrypts watch requests and signs nothing.
2. **Host identity key: Ed25519, generated by the host** (team-store.ts:442-446, `generateKeyPairSync("ed25519", … pem)`). It is registered with the account service as `devicePublicKey` (host-service.ts:1005). It signs the host's side of the WebRTC authentication transcript (`this.#store.signRemoteAuthentication(responseTranscript)`, team-webrtc-host-peer.ts:531). It also signs key handovers: both the old and new keys sign `["openbot-host-key-handover-v1", hostId, issuedAt, previousPublicKey, publicKey]` (contracts/host-key-handover.ts:18-28).
3. **Client connection key: a fresh Ed25519 key per connection attempt, generated by the joining desktop** (team-webrtc-client-transport.ts:946-952). Its public half is sent to the account service in `POST /v2/remote/sessions/:id/ticket` with `{clientPublicKey}` (M:1103-1120). The account service binds it into the JWT.
4. **Watch client key: ECDSA P-256 on the watch or phone** (inferred from `verifyWatchProof`, which uses `{name:"ECDSA", namedCurve:"P-256"}`, contracts/signal-protocol/watch-relay.ts:102-121). It arrives as `clientPublicKey` inside the watch JWT.

### How a remote member's identity reaches the host and is checked
- **The account service is the identity authority.** It issues ES256 JWTs. The host verifies them with `jose` against a JWKS fetched from the same account service (M:1201-1240):
  ```ts
  return yield* authCall(() => jwtVerify(ticket, keySet, { audience, algorithms: ["ES256"] }));
  ```
  - The JWKS is cached forever after the first successful fetch (`Duration.infinity`, M:1209). It is dropped and refetched when no key matches (`ERR_JWKS_NO_MATCHING_KEY`, M:1216-1221), so the service can add or rotate signing keys at any time.
  - There is no `issuer` check, only audience and algorithm. `jose` still enforces `exp`/`nbf` by default.
  - The JWKS origin is the account-service URL, so it is only as trusted as TLS to `api.openbot.run`.
- **Remote session ticket** (audience `REMOTE_TICKET_AUDIENCE`). The required claims are `sessionId, hostId, userId, membershipId, role ∈ {owner,admin,member}, authEpoch, sessionExpiresAt, clientPublicKey` (M:1122-1156). The host peer then:
  - checks that the claims match the pending Signal connection, including `claims.clientPublicKey === frame.clientPublicKey` (team-webrtc-host-peer.ts:481-493);
  - checks the client's **Ed25519 proof of possession** over a transcript that includes the ticket, nonces, and **both DTLS fingerprints** (team-webrtc-host-peer.ts:494-509). This binds the account-service-vouched identity to this specific WebRTC/DTLS channel;
  - replies with a nonce and a signature from its host identity key (team-webrtc-host-peer.ts:510-531).
- **Watch ticket** (audience `openbot-watch`, 10-minute TTL per contracts watch-relay.ts:6-8). The claims checked are `jti, watchSessionId, hostId, userId, membershipId, role, authEpoch, clientPublicKey, signal?, iat, exp` (M:1158-1199).
- **Membership comes from the ticket, with a fallback.** watch-relay-service.ts:120 has `this.#options.member(claims.membershipId, claims.role) ?? ticketMember(claims)`. When the host's local team store does not know the member, `ticketMember` builds one from the JWT claims with `disabled: false` (watch-relay-service.ts:235-245). In application-services.ts:3352-3355, an `owner` role claim also falls back to the local owner record. The host's local checks are `claims.hostId === hostId` and `claims.authEpoch === ingress.authEpoch()` (watch-relay-service.ts:117-119). The epoch lets the account service invalidate all tickets for a host.
- **How clients trust the host key** (packages/team-client/src/host-key-handover.ts, followed briefly):
  - The fingerprint is `sha256(publicKeyPEM)`, base64url, 43 characters (lines 12-15).
  - A client that holds a `trustedFingerprint` follows a chain of handovers in which both the previous and next keys must sign (lines 17-36). This is a key-continuity pin.
  - The *initial* `devicePublicKey` is delivered by the account service: in `listRemoteHosts` (records:30, 175) and in the invite preview (records:64, 252-253). First contact therefore trusts the account service, except in Mobile Connect, where the QR code carries the fingerprint (section 4).

---

## 3. Data at rest

### secret-storage-cipher.ts
- **Primary:** Electron `safeStorage.encryptString` / `decryptString` (secret-storage-cipher.ts:130-133, 149-150). That is macOS Keychain, Windows DPAPI, or Linux libsecret/kwallet.
- **Fallback (Linux and macOS only, `keyFileFallback = linux || platform === "darwin"`, line 45):**
  - A 32-byte random key is written base64 to `userData/secret-storage.key` (`SECRET_STORAGE_KEY_FILE`, line 18; account-service-access.ts:19).
  - The file is created `0o600` with a temp file plus `linkSync`, then fsync'd (lines 61-98).
  - The cipher is **AES-256-GCM** with a 12-byte IV and 16-byte tag. The format is `"obk1" || iv || tag || ciphertext` (lines 20-23, 134-137).
- **When the fallback is used:**
  - **macOS:** only when `safeStorage.isEncryptionAvailable()` is false (line 102).
  - **Linux:** whenever the key file exists. Line 102's availability short-circuit is skipped on Linux, so once a key file exists it wins for all *new* writes even if the keyring comes back. The key file is created when safeStorage is unavailable (lines 105-106). It is also created after any Linux `safeStorage.decryptString` failure (lines 151-159). In that case the current decrypt still throws, and the user lands in `signed_out` with reason `keyring_changed` (M:1740, 1749).
  - **Windows:** there is no fallback. Encryption throws `unavailableMessage` (line 131).
- **What the fallback actually protects:** the key file sits in the same `userData` directory as the ciphertext, protected only by the `0600` file mode. Against anyone who can read the user's files, this is equivalent to plaintext. The logged message matches the code: "The system keyring did not keep the secrets. OpenBot keeps new secrets in its own key file." (line 93).
- Caveat from Electron behavior, not from this code: on Linux without a keyring, Electron can select the `basic_text` backend, which uses a hardcoded key, and still report encryption as available. This code logs `getSelectedStorageBackend()` (line 95) but does not reject `basic_text`.

### What is stored, and where (all under Electron `userData`)
| File | Contents | Protection |
|---|---|---|
| `openbot-central-auth-v1.bin` (account-service-access.ts:13, 60) | base64 of `encrypt(JSON {version:2, sessionToken, teamHostTokens:{hostId→machineToken}, mobileConnectLink?:{host,token,createdAt}, user?})` (M:2584-2591) | cipher above. Written `0o600` via tmp + rename (M:2593-2596). If storage is unavailable, the session is kept in memory only; with `required`, the file is deleted and the call fails (M:2573-2579). A legacy format stored just the raw token string (M:2638-2643). |
| `openbot-watch-relay-key-v1.json` | `{version:1, publicKey:<raw P-256 b64url, plaintext>, privateKey:<base64(cipher.encrypt(pkcs8 b64url))>}` (stored-p256-key.ts:42-43) | private key encrypted with the cipher. Written with `writeFileAtomically` (file mode not visible here). If encryption fails, no key is created and the watch relay is unavailable (lines 45-48). |
| watch-relay "wanted" preference (`WATCH_RELAY_WANTED_FILE`) | `{version:1, wantedUntil}` (watch-relay-key.ts:23-35) | plaintext |
| `secret-storage.key` | fallback AES key | `0600` only |

- Secret values are registered for log redaction (`registerSecretValue`, stored-p256-key.ts:35, 41).
- Not covered here: the host Ed25519 private key in team-store (PEM). I did not check whether team-store encrypts it.

---

## 4. Watch relay and Mobile Connect

### Watch relay (Apple Watch to host)
**Purpose.** An Apple Watch (or the phone on its behalf) sends commands to the host and gets back a compact snapshot. The commands are refresh, avatars, read, approve, deny, answer, send, stop, sendChannel, and readChannel. `pair` and `open` are refused (watch-relay-commands.ts:177).

**Path.** Watch → Signal relay (`/v1/watch/request`, contracts watch-relay.ts:12; served by `remote/api`) → host via signal ingress (`WatchIngressDelivery {ticket, ts, sealed, proof}`). The host holds a lease on the relay while the watch is "wanted": `acquireWatchRelay`, for up to 30 days per `WATCH_SESSION_TTL_MS` (watch-relay-service.ts:194-227; contracts:10).

**Host checks**, in order (watch-relay-service.ts:106-151):
1. JWT verified by the account service JWKS (401 on failure).
2. `hostId` matches (403).
3. `authEpoch` matches (401).
4. Member not disabled (403).
5. Clock within ±5 minutes (`watchClockAccepts`, contracts:238-240).
6. ECDSA P-256 proof by `claims.clientPublicKey` over `JSON[ "openbot-watch-request-v1", hostId, jti, sha256(sealed), ts ]` (contracts:89-91, 97-121).
7. Open the sealed request.
8. Replay check on the inner `request.id`: 10-minute window, up to 10,000 entries (watch-relay-service.ts:31-32, 174-183).

**Encryption, end to end between the watch and the host:**
- **Request:** ECIES with the host's static `watchKey` (contracts/host-grant.ts:38-124):
  - an ephemeral ECDH P-256 key pair, then `HKDF-SHA256(salt = ephemeral public key, info = "openbot-watch-request-v1")`, then AES-256-GCM;
  - the additional authenticated data (AAD) is `"hostId\njti"` (contracts watch-relay.ts:30, 85-87, 123-145);
  - the envelope is `{v, epk, iv, ct}`.
- **Reply:** the request contains a 32-byte `responseKey` chosen by the watch (contracts:159-160). The host replies with AES-256-GCM under that key with AAD = `requestId` (contracts:177-191; watch-relay-service.ts:185-192). Replies are size-capped and shrunk to fit (watch-relay-service.ts:36, 247-264; snapshot.ts:461-483).
- **No forward secrecy on the host side.** The host's key is static. Whoever gets `openbot-watch-relay-key-v1.json` plus the cipher key can decrypt recorded requests, and through the `responseKey` inside each request, the replies too.
- **What the relay sees:** the JWT ticket (hostId, userId, membershipId, role, watchSessionId), timestamps, the proof, ciphertext sizes, and timing. It cannot read commands or snapshots.
- **Where the watch learns `watchKey`:** from `setup()` → `{hostId, watchKey}` (watch-relay-service.ts:96-104) and from `snapshot.servers[].watchKey` (snapshot.ts:446-455). These are delivered over the team protocol (contracts `team-protocol/watch-relay-v1`), not via the account service, as far as these files show.
- **Reply contents** (snapshot.ts:400-459): agent names and states, latest reply text, approval command text, cwd and file paths with `/Users/x` shortened to `~` (lines 101-102, 167-169), questions (secret questions are pushed to "phone", lines 211-214), channel names and last-message previews, server name and appearance, and avatar images as base64 (watch-relay-commands.ts:339-366).
- **Execution:** each command runs as the member through a **loopback HTTP call** to the host's own Team API at `http://127.0.0.1:${port}`. A 60-second session is opened just for that call (watch-relay-commands.ts:368-417). So the normal Team API permission checks apply.
- **Approval guard:** approve is allowed only if the watch saw the *whole* approval text (`agent.wait.whole`, watch-relay-commands.ts:208-212; snapshot.ts:204). Secret questions can't be answered from the watch (watch-relay-commands.ts:224-231).
- **Comment/string claims vs code:**
  - The warning strings say "The Apple Watch uses the iPhone to reach this host" when the key cannot be encrypted or saved (watch-relay-key.ts:13-14). The code just returns `null`, and `setup()` then returns `null` (watch-relay-service.ts:101). The phone fallback path is not in these files.
  - When the member is not found locally, the host builds one from the ticket claims (`ticketMember`), so access then rests entirely on the account-service assertion.

### Mobile Connect (pair a phone with this desktop's server)
- **Host side** (mobile-connect-host.ts:28-89): it makes sure the host is configured (default name "OpenBot"), started, `online`, `apiOnline`, and has a `ws:`/`wss:` `apiUrl`. It then takes the binding `{hostId, fingerprint}`, where `fingerprint = sha256(host identity public key)` (host-service.ts:843-846; team-client host-key-handover.ts:12-15). If the account service answers `mobile_host_mismatch`, the host re-registers and retries once (lines 42-63).
- **Ticket:** `POST /v1/mobile-auth/ticket` with `{host: {hostId, fingerprint}}` returns `{ticket, expiresAt}`. Expiry is corrected using the server's `Date` header (M:414-442, 2708-2717).
- **QR/URL:** `openbot://mobile-connect?api=<origin>&ticket=<…>&host=<hostId>&fingerprint=<…>` (contracts/mobile-connect.ts:70-83). The API origin must be HTTPS, or HTTP on loopback or a private LAN address (lines 110-139).
- **Persistent link:** `POST /v1/mobile-auth/link` returns a token. It is stored in the encrypted session file, and can be revoked with DELETE (M:472-519).
- **What crosses the network:** the ticket goes through the account service. The phone redeems it there and gets its own account session; the redeem endpoint is server-side and not in these files. `validateMobileConnectHostBinding` (contracts/mobile-connect.ts:19-31) makes the redeemed result match the QR's hostId and fingerprint: "The redeemed Mobile Connect code belongs to a different desktop identity." The QR is therefore an out-of-band channel for the host-key fingerprint that the account service cannot silently swap, assuming the mobile app enforces it (not verified here).
- **Device management:** list and revoke mobile devices and account sessions (M:532-601). All of it is held by the account service.

---

## 5. End-to-end encryption, or trust in TLS plus the account service?

- **Host ↔ member transport:** WebRTC/DTLS through Signal (inferred from the DTLS fingerprints in team-webrtc-host-peer.ts:500-501, 518-519). It is encrypted end to end between the two peers, and the authentication transcript binds both DTLS fingerprints. The host proves itself with its Ed25519 identity key. The client proves possession of a per-connection Ed25519 key.
- **Watch ↔ host:** application-layer E2E (ECIES request plus AES-GCM reply). The relay cannot read the content.
- **Who vouches for whom:**
  - **Member identity, role, and membership:** only the account service, via ES256 JWTs checked against a JWKS fetched over TLS from that same service. There is no issuer check and the JWKS is refreshed on demand. Anyone who controls the account service or its signing key can mint a ticket for any user and role on any host. For the watch relay, the host will even build a member record from such a ticket. The host cannot independently tell whether a remote person is who the account service says.
  - **Host identity:** the client pins a key fingerprint and follows dual-signed handovers. The initial key comes from the account service (`devicePublicKey`), except in Mobile Connect, where it comes from the QR code.
- **No end-to-end encryption between people:** I found no per-user or group content keys. The host decrypts and holds everything. Push notifications, Live Activity updates, integration routes, invite emails, and the GitHub token pass through the account service as JSON over TLS.
- **Data at rest:** the OS keychain via Electron safeStorage. On Linux and macOS, if the keychain is unavailable, the fallback is a plaintext-equivalent key file stored next to the data.

**Bottom line:** transport confidentiality is end to end (DTLS for teams, ECIES for the watch). Authentication and authorization rest on TLS to `api.openbot.run` plus the account service's JWT signing key and membership database. Mobile Connect's QR fingerprint and the host-key handover chain are the only out-of-band or continuity anchors.
