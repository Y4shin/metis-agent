---
type: task
subtype: grilling
title: Authentication scheme and credential transport between surfaces and the daemon
status: stable
workflow_state: done
blocked_by: []
mode: human
---

## Decision to settle

How a client surface authenticates to the daemon and how the `client` crate
carries that credential:

- the scheme: bearer token, signed request, mutual TLS, OS-level trust, or
  none over local IPC;
- what local IPC trust means: whether a local connection is implicitly
  trusted, or also presents a credential;
- credential issuance: first-run pairing, a daemon-minted token, or a
  user-configured secret;
- credential storage in the client: file, OS keychain, Android Keystore;
- scopes: per surface, per agent, or global, and whether the daemon resolves
  agent permissions from the agent context instead;
- transport of the credential for HTTP and for the WebSocket upgrade;
- failure behaviour on missing, expired, revoked, or rotated credentials;
- whether the client's endpoint configuration (transport selection, base
  path, TLS trust) is decided here or only the credential half.

## Parent decisions

Already settled and not to be reopened:

- One endpoint implementation is shared by the local IPC listener and the
  optional TCP/TLS listener; local IPC is the default and remote access is
  explicitly enabled.
- `metis-daemon` owns configuration and secret resolution.
- The `client` crate is UI-independent and shared by `terminal`, the Tauri
  app, and Android.
- The walking skeleton proves the transports without authentication.

## Choices already known

- Local IPC is already constrained to the intended users by Unix socket
  permissions and Windows pipe security descriptors, so local trust can be
  implicit.
- Remote access is opt-in and explicit, which makes it the natural place for a
  real credential.
- Android cannot rely on a filesystem permission as its trust anchor.
- Agent permissions are resolved server-side from the agent context, so a
  token does not have to encode them.

## Decisions recorded

**D1, local IPC authentication.** The host filesystem ACL is the
authentication. On Unix the daemon owns the socket and its directory as a
configured `uid:gid` with mode 700 or 770 chosen by configuration. On Windows
the pipe security descriptor covers the intended users. The connection
handshake still declares the protocol version, the surface name, and the client
version, for logging and surface identity. It is not authentication.

**D2, remote credential.** A per-surface opaque token minted at pairing. The
daemon stores only its hash, in its own state. Revocation removes the entry and
requires a restart, consistent with the startup-read rule.

**D3, scopes.** A client credential grants the full surface the daemon exposes.
There are no per-client permissions: every paired client can reach every agent
and personality. Agent permissions are resolved server-side from the agent
context.

**D4, credential transport.** `Authorization: Bearer` on every RPC, including the
event stream, carried as request metadata. The credential never appears in a URL.

**D5, client storage.** Best effort against the platform keychain by invoking
its CLI, falling back to a file with owner-only permissions. Android uses its
platform keystore.

**D6, endpoint configuration.** One client profile record holding the transport
kind (local by default, with the platform default path or the environment
override, or a remote URL), the credential, and the optional pin.

**D7, pin form.** The server's public key fingerprint (SPKI), so a certificate
renewal that reuses the key keeps working.

**D8, preshared key source.** The operator sets the pairing preshared key as a
secret value in the daemon configuration (literal, `$ENV:`, `$KEYSTORE`, or
`$SECRET_FILE:`). The daemon accepts it for pairing while it remains configured,
so rotating or removing it closes pairing. The `$SECRET_FILE:` form is new for
this task and is recorded on the `metis-daemon` map: a path relative to the
config file, whose permissions must grant the owner read access and grant group
and others nothing, otherwise the file is rejected.

**D10, token lifetime.** No expiry in V1. Revocation is the only invalidation,
and an expiry or refresh mechanism may be added later as an additive change.

**D11, failure behaviour.** A missing credential is a typed unauthenticated
error, an invalid or revoked one is a distinct typed error, there is no
downgrade to anonymous access, and revocation takes effect on restart.

**D12, handshake contents.** Protocol version, surface name, and client version
string. No capability negotiation at connect time.

**D9, pairing availability.** Pairing is reachable on both the remote listener and
local IPC, gated by the preshared key, so a network client such as Android pairs
itself. Closing pairing is done by removing the preshared key from the
configuration and restarting.

## Alternatives considered

- A credential on local IPC as well. Rejected: the OS user is already the trust
boundary, and a second check cannot strengthen it.
- A signed stateless token. Rejected: per-device revocation needs server state,
and that state is one list of hashes.
- The preshared key used directly as the credential. Rejected: no per-device
revocation.
- One credential per agent, or one global credential. Rejected: per-surface
credentials keep revocation meaningful, and agent permissions already resolve
server-side.
- The credential in a query parameter or in a post-connect WebSocket message.
Rejected: the header rule covers both transports and keeps credentials out of
URLs and access logs.
- The OS keystore on every platform. Rejected for desktop: shelling out to the
platform CLI with a file fallback fits the static-linking preference.
- A certificate-fingerprint pin. Rejected in favour of the public-key (SPKI) pin,
which survives a renewal that reuses the key.
- Token expiry and refresh tokens. Rejected for V1: no expiry, revocation only,
and an expiry mechanism can come later as an additive change.
- One generic forbidden error. Rejected: the client must tell "pair me" from
"you are no longer welcome".
- Pairing over local IPC only, or behind an operator-opened window. Rejected in
favour of pairing on both transports, gated by the preshared key, which reaches
the window's end state by removing the key.

## Dependent-task implications

- The contract carries the pairing operation and the typed authentication
errors, and the parity rule applies to both. The connection handshake fields
carry the protocol version, surface name, and client version.
- `metis-daemon`: the token-hash store, the pairing operation, the socket ACL
configuration (owner and mode, or the Windows descriptor), and the interaction
between the remote listener and reverse-proxy mode.
- `client`: the profile record (transport, token, pin), the keychain CLI attempt
with its file fallback, the `Authorization` header on HTTP and on the WebSocket
upgrade, and handling of the distinct auth errors.
- `metis-surfaces`: pairing UX on desktop and Android, including preshared key
entry and fingerprint confirmation.
- The preshared key is a configuration secret, so it uses the
`$ENV:`/`$KEYSTORE`/`$SECRET_FILE:` mechanism.
- Windows local trust is the pipe security descriptor, which ties this task to
the `windows-pipe-security` findings.

## Remaining fog

- The exact ACL configuration shape: an owner and mode, or a named group, and its
Windows equivalent.
- Where the daemon stores its token hashes and with what file permissions.
- The pairing message shape, and whether it needs a nonce or rate limit against
preshared key guessing.
- Whether the client warns when it falls back from the platform keychain to a
file.
- Whether the surface name and client version strings are validated or
free-form.

## Completion evidence

Recorded above: decisions D1 to D12, the alternatives considered, the
dependent-task implications, and the remaining fog.

## Later amendment

The transport is now ConnectRPC rather than HTTP with server-sent events, per
`realtime-semantics` D16. The credential still travels as an `Authorization`
header, which Connect carries as request metadata, so D4 holds in substance. The
connection handshake in D12 becomes connection metadata.
