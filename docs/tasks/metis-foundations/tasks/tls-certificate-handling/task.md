---
type: task
subtype: grilling
title: Certificate and key handling for the optional TCP/TLS listener
status: stable
workflow_state: done
blocked_by: []
mode: human
---

## Decision to settle

How the optional TCP/TLS listener obtains, stores, serves, rotates, and
trusts certificates and keys, and what the clients accept:

- the TLS implementation (rustls, native-tls, or other) and why;
- where the certificate and key come from: generated on first enable,
  operator-provided PEM files, ACME, or a native store;
- where the private key lives (file permissions, keychain) and who may read
  it;
- what the client trusts: an explicit CA, a pinned fingerprint or public key,
  the OS trust store, or a combination;
- how a client (including the Android app) receives that trust material, given
  a personal host with no public DNS name;
- rotation and expiry: what happens on renewal, and whether a changed
  certificate fails closed or re-prompts;
- how enablement is expressed (an explicit flag plus a bind address) without
  contradicting the local-IPC default.

## Parent decisions

Already settled and not to be reopened:

- The TCP/TLS listener is off unless explicitly enabled; local IPC is the
  default desktop transport.
- One endpoint implementation serves both local and remote transports.
- The endpoint path for local IPC is the platform default with an environment
  variable override.
- The client owns authentication and endpoint configuration.
- `metis-daemon` owns configuration and secret resolution, so this task
  decides the TLS shape, not the configuration mechanism.

## Choices already known

- rustls avoids OpenSSL and works on Android; native-tls follows the platform
  store. This is an open choice.
- Deployment is personal and self-hosted, usually without a public name, so
  public CAs and ACME are a poor fit for the common case.
- Android remote access uses the network transport, so the Android app needs
  the same trust decision as the desktop client.
- The map's Constraints require that remote access stays off by default.

## Decisions recorded

**D1, TLS implementation.** rustls on daemon and client, with a statically
linked crypto provider. Constraint: avoid dynamically linked system libraries,
preferring static linking for the daemon and its crates. Native plugins are
excluded, since a shared library is what a plugin is.

**D2, certificate source.** An operator-provided PEM certificate and key, given
as a path or a configuration secret reference, is the supported path, with
first-enable self-signed generation as a convenience. ACME is not implemented in
the daemon; a reverse proxy owns publicly trusted certificates.

**D3, secret values.** A secret configuration value is a literal, an
environment variable reference (`$ENV:NAME`), or an OS keystore reference
(`$KEYSTORE`) resolved to the keystore item whose canonical name is the config
item's path. This applies to every secret and is implemented by `metis-daemon`
configuration resolution; it is recorded on that map.

**D4, client verification.** Check the chain against the OS trust store first;
if that fails, accept only when the presented certificate matches the stored
pin from pairing. Hostname verification is always on. There is no CA-file
configuration.

**D5, no certificates for client trust.** No client certificates and no mutual
TLS. The daemon authenticates clients with the credential and pairing.

**D6, plaintext TCP.** Plaintext is allowed on the TCP listener. A warning is
emitted when the listener is not loopback, is plaintext, and reverse-proxy mode
is not declared.

**D7, reverse-proxy mode.** An explicit configuration block naming trusted proxy
addresses. Forwarded headers are honoured only from those peers, and declaring
it suppresses the plaintext warning. The daemon is expected to bind loopback in
this shape.

**D8, bind address.** No default. The operator states the address explicitly.

**D9, read at startup.** TLS material and configuration are read once at
startup; changing either requires a restart. No file watching, no hot reload,
no certificate directory scanning.

**D10, pairing.** Gated by a preshared key. It delivers the credential and, when
the listener's certificate is not publicly trusted, the fingerprint pin. The
pairing flow itself belongs to `authentication-scheme`.

## Alternatives considered

- native-tls or OpenSSL. Rejected: a dynamically linked system dependency and a
host-version dependency on the daemon.
- ACME in the daemon. Rejected: a reverse proxy owns publicly trusted
certificates.
- An explicitly configured CA file. Rejected as unnecessary complexity given
the OS store plus pin rule.
- Mutual TLS. Rejected: credentials, not certificates, authenticate clients.
- Hot reload and certificate directory scanning. Rejected as overengineering.
- Forbidding plaintext TCP. Rejected: it conflicts with reverse-proxy
deployment.
- A default bind address. Rejected: explicit is required.
- Trust on first use without pairing. Rejected: pairing supplies the pin.
- Always requiring the pin and skipping the OS store. Rejected: an automatic
proxy certificate rotation would break every paired client.

## Dependent-task implications

- `authentication-scheme` owns the pairing flow: the preshared key, credential
issuance, and the fingerprint exchange.
- `metis-daemon`: bind address, certificate and key references, the
reverse-proxy block, startup-only configuration reading, and the plaintext
warning at startup.
- `client`: a rustls connector that checks the OS store first and falls back to
the pin, storing the pin beside the credential, with no CA-file option.
- `metis-surfaces`: pairing UX on desktop and Android, and no certificate UI.
- Build and CI: the rustls crypto provider and the static-linking target for the
daemon.

## Remaining fog

- Which rustls crypto provider (`ring` or `aws-lc-rs`) and which static-linking
target, which is a build decision for `metis-daemon` and CI.
- Where the client stores the pin, presumably beside the credential.
- Whether the daemon prints its certificate fingerprint for pairing, which the
pairing flow decides.
