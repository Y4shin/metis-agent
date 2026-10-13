# 0004. The daemon does not terminate TLS; a reverse proxy does

- Status: accepted
- Date: 2026-10-13

## Context

The effort's constraints first said the remote transport would use rustls on
both sides, with TLS material read at startup, and a `tls-and-client-trust`
ticket was charted for it. The operator's decision on 2026-10-13 is the
opposite: the daemon does not support TLS natively. If the remote listener is
exposed beyond a trusted network, a reverse proxy terminates TLS in front of it,
and the daemon's remote transport stays plaintext.

The reasons: certificate and key lifecycle inside the daemon is configuration
surface with no product value when a proxy already owns the deployment boundary,
and client-side trust (a pinned fingerprint learned at pairing, or the operating
system store) needs rethinking from the ground up rather than being inherited
from an early sketch. Client trust is therefore deferred past the first efforts.

## Consequences

- The remote listener is plaintext and remains off by default with a required
  bind address. Exposure without a proxy is an operator error the daemon cannot
  detect.
- The preshared key and minted tokens can be observed on a plaintext wire, so
  pairing and token use over the remote listener rest on the proxy and on the
  network, not on the daemon.
- `reverse-proxy-mode` owns the trusted-proxy rule and the plaintext warning that
  the map's constraints require; the daemon does not grow a certificate, key or
  trusted-CA setting.
- `tls-and-client-trust` is deprecated and out of the graph, and the map's rustls
  constraint is replaced by this rule.
- Revisiting this means reopening client trust as its own decision, with the
  proxy deployment in mind, rather than restoring the deprecated ticket.
