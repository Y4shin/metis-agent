---
type: task
subtype: research
title: Windows named pipe security descriptor and the exact remote-client rejection path
status: stable
workflow_state: done
blocked_by: []
---

## Question

Which exact Win32 mechanism creates the daemon's named pipe listener so that
only the intended local users can connect and remote pipe clients are
rejected, and which Rust API exposes it? Concretely:

1. The security descriptor or SDDL for the intended users, and what "intended
   users" means in practice for a per-user personal daemon (the interactive
   user only, or a service account, or a group).
2. The remote-client rejection path: whether `PIPE_REJECT_REMOTE_CLIENTS` is
   the supported mechanism, how a remote attempt fails, and whether anything
   else is needed to make the pipe local-only.
3. Whether the chosen cross-platform crate can pass `SECURITY_ATTRIBUTES` to
   pipe creation, or whether the Windows transport has to fall back to raw
   Win32 (`windows-sys` or `windows`) behind the listener adapter.
4. Client-side consequences: the pipe name and path form, impersonation or
   identity checks, and what the connector does when it is rejected.

## Decision this unblocks

The Windows transport adapter in `metis-daemon`'s listener and the
`metis-foundations` connector, and the map constraint that Windows pipes are
local-only with security descriptors for the intended users and remote pipe
clients rejected. It also feeds the endpoint configuration decision already
made (platform default with an environment variable override).

## Trusted source boundaries

- Primary: Microsoft Win32 documentation for `CreateNamedPipe`,
  `SECURITY_ATTRIBUTES`, `SECURITY_DESCRIPTOR`, SDDL, the pipe mode flags, and
  named pipe impersonation.
- Primary: the documentation and source of the candidate Rust crates
  (`interprocess`, `tokio::net::windows::named_pipe`, `windows-sys`, and any
  maintained wrapper that exposes security attributes).
- Verification: a Windows CI runner or a documented manual check on a Windows
  host. Secondary write-ups may point at candidates but every claim has to be
  confirmed against the primary source.

## Evidence required for completion

- The concrete descriptor or SDDL string for the intended-user-only case, with
  the documentation that justifies each access right.
- Confirmation of the remote-client rejection mechanism, including how the
  client observes the rejection.
- The API surface in the chosen crate, or a documented raw Win32 fallback with
  the wrapper shape the listener adapter needs.
- Any session-boundary caveats (other sessions on the same machine, low
  integrity processes, the `\\?\pipe\` path form) that the intended-user
  descriptor does not cover.
- A reproduction, or an explicit statement that no Windows runner is
  available, plus the minimal verification plan that would close the gap.
  Local development here is Linux, so an unverified claim must be labelled as
  such rather than presented as tested.
- Every material claim cited to a URL or a repository path.

## Dependent tasks

The Windows listener and connector implementation, the endpoint configuration
work, and any CI matrix addition needed to run the Windows transport tests.
