---
type: findings
title: Windows named pipe security descriptor and the exact remote-client rejection path
status: stable
---

# Research: Windows named pipe listener security and remote-client rejection

## Summary

The daemon's Windows listener is `CreateNamedPipeW` with an explicit
`SECURITY_ATTRIBUTES` whose `lpSecurityDescriptor` is an SDDL-built descriptor
granting the daemon's own user SID only, plus `PIPE_REJECT_REMOTE_CLIENTS`
(0x00000008) in `dwPipeMode`. The descriptor hides the pipe from other
accounts; the mode flag is what makes it local-only, because a DACL keyed on a
user SID does not exclude a remote caller who authenticated as that same user.
In Rust, `tokio::net::windows::named_pipe` exposes exactly this: remote clients
are rejected by default (`ServerOptions::reject_remote_clients` defaults to
`true`) and `ServerOptions::create_with_security_attributes_raw` passes a raw
`SECURITY_ATTRIBUTES` pointer into `CreateNamedPipeW`, so no `interprocess`
dependency is required. Nothing in this brief was executed or compiled on
Windows; the development host here is Linux.

## Question investigated

Which exact Win32 mechanism creates the daemon's named pipe listener so that
only the intended local users can connect and remote pipe clients are rejected,
and which Rust API exposes it? Concretely (per
`docs/tasks/metis-foundations/tasks/windows-pipe-security/task.md`):

1. The security descriptor or SDDL for the intended users, and what "intended
   users" means for a per-user personal daemon.
2. The remote-client rejection path: is `PIPE_REJECT_REMOTE_CLIENTS` the
   supported mechanism, how does a remote attempt fail, is anything else
   needed to make the pipe local-only.
3. Whether the candidate cross-platform crate can pass `SECURITY_ATTRIBUTES`,
   or whether the Windows transport needs a raw Win32 fallback.
4. Client-side consequences: pipe name and path form, impersonation or identity
   checks, and what the connector does when it is rejected.

Constraints from the task: pipes are local-only with descriptors for the
intended users; the daemon implements the listener and the client crate the
connector; avoid dynamically linked system libraries. Repository:
`/home/patric/Projects/metis-agent` (see
`docs/tasks/metis-foundations/map.md` constraints and `docs/rough-plan.md`,
"Local and remote transports").

## Method and evidence limits

Local development is Linux and no Windows runner was available for this task.
Every Windows-specific statement below is documentation or crate source reading,
not a test result. Two claims are deliberately left as unverified and are
listed under "Missing evidence": the exact error a remote client observes, and
whether the named pipe file system applies Mandatory Integrity Control to pipe
objects.

## Sources and quoted passages

### Microsoft Win32

`CreateNamedPipeW` (https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-createnamedpipew)

> `lpName`: "The unique pipe name. This string must have the following form:
> `\\.\pipe\pipename` ... The entire pipe name string can be up to 256
> characters long. Pipe names are not case sensitive."

> `lpSecurityAttributes`: "A pointer to a `SECURITY_ATTRIBUTES` structure that
> specifies a security descriptor for the new named pipe and determines whether
> child processes can inherit the returned handle. If `lpSecurityAttributes` is
> `NULL`, the named pipe gets a default security descriptor and the handle
> cannot be inherited. The ACLs in the default security descriptor for a named
> pipe grant full control to the LocalSystem account, administrators, and the
> creator owner. They also grant read access to members of the Everyone group
> and the anonymous account."

> Remote-client modes: "PIPE_ACCEPT_REMOTE_CLIENTS, 0x00000000: Connections
> from remote clients can be accepted and checked against the security
> descriptor for the pipe." / "PIPE_REJECT_REMOTE_CLIENTS, 0x00000008:
> Connections from remote clients are automatically rejected."

> First-instance flag: "FILE_FLAG_FIRST_PIPE_INSTANCE, 0x00080000: If you
> attempt to create multiple instances of a pipe with this flag, creation of the
> first instance succeeds, but creation of the next instance fails with
> ERROR_ACCESS_DENIED."

> `dwOpenMode` security modes include "ACCESS_SYSTEM_SECURITY, 0x01000000L: The
> caller will have write access to the named pipe's SACL."

> Remarks: "To create an instance of a named pipe by using CreateNamedPipe, the
> user must have FILE_CREATE_PIPE_INSTANCE access to the named pipe object. If a
> new named pipe is being created, the access control list (ACL) from the
> security attributes parameter defines the discretionary access control for
> the named pipe."

> App-container note: "Windows 10, version 1709: Pipes are only supported
> within an app-container; ie, from one UWP process to another UWP process
> that's part of the same app. Also, named pipes must use the syntax
> `\\.\pipe\LOCAL\` for the pipe name."

`Named Pipe Security and Access Rights` (https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights)

> "You can specify a security descriptor for a named pipe when you call the
> CreateNamedPipe function. The security descriptor controls access to both
> client and server ends of the named pipe."

> "When a thread calls CreateNamedPipe to open a handle to the server end of an
> existing named pipe, the system performs an access check before returning the
> handle. The access check compares the thread's access token and the requested
> access rights against the DACL in the named pipe's security descriptor. In
> addition to the requested access rights, the DACL must allow the calling
> thread FILE_CREATE_PIPE_INSTANCE access to the named pipe. Similarly, when a
> client calls the CreateFile or CallNamedPipe function to connect to the client
> end of a named pipe, the system performs an access check before granting
> access to the client."

> "FILE_GENERIC_WRITE access for a named pipe combines the rights to write data
> to the pipe, append data to it, write pipe attributes, write extended
> attributes, and read the pipe's DACL. Because FILE_APPEND_DATA and
> FILE_CREATE_PIPE_INSTANCE have the same definition, so FILE_GENERIC_WRITE
> enables permission to create the pipe. To avoid this problem, use the
> individual rights instead of using FILE_GENERIC_WRITE."

> "To prevent remote users or users on a different terminal services session
> from accessing a named pipe, use the logon SID on the DACL for the pipe. The
> logon SID is used in run-as logons as well; it is the SID used to protect the
> per-session object namespace."

`Named Pipes` (https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes)

> "Named pipes can be used to provide communication between processes on the
> same computer or between processes on different computers across a network.
> If the server service is running, all named pipes are accessible remotely. If
> you intend to use a named pipe locally only, deny access to NT
> AUTHORITY\NETWORK or switch to local RPC."

`Pipe Names` (https://learn.microsoft.com/en-us/windows/win32/ipc/pipe-names)

> "Use the following form when specifying the name of a pipe in the CreateFile,
> WaitNamedPipe, or CallNamedPipe function: `\\ServerName\pipe\PipeName` where
> ServerName is either the name of a remote computer or a period, to specify the
> local computer."

`Named Pipe Client` (https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-client)

> "If the pipe exists but all of its instances are busy, CreateFile returns
> INVALID_HANDLE_VALUE and the GetLastError function returns ERROR_PIPE_BUSY."

`Impersonating a Named Pipe Client` (https://learn.microsoft.com/en-us/windows/win32/ipc/impersonating-a-named-pipe-client)

> "A named pipe server thread can call the ImpersonateNamedPipeClient function to
> assume the access token of the user connected to the client end of the pipe."

`Security Descriptor String Format` (https://learn.microsoft.com/en-us/windows/win32/secauthz/security-descriptor-string-format)

> "`D:dacl_flags(string_ace1)(string_ace2)...` ... `P` | SDDL_PROTECTED | The
> SE_DACL_PROTECTED flag is set."

`ACE Strings` (https://learn.microsoft.com/en-us/windows/win32/secauthz/ace-strings)

> Generic access rights: "`GA` | SDDL_GENERIC_ALL | GENERIC_ALL"; `GR` =
> GENERIC_READ, `GW` = GENERIC_WRITE. Mandatory label rights: "`NW` |
> SDDL_NO_WRITE_UP | SYSTEM_MANDATORY_LABEL_NO_WRITE_UP". Example ACE string form:
> `(A;;RPWPCCDCLCSWRCWDWOGA;;;S-1-1-0)`.

`SID Strings` (https://learn.microsoft.com/en-us/windows/win32/secauthz/sid-strings)

> "`NU` | SDDL_NETWORK | Network logon user. This is a group identifier added to
> the token of a process when it was logged on across a network. The
> corresponding logon type is LOGON32_LOGON_NETWORK."

> "`SY` | SDDL_LOCAL_SYSTEM | Local system."; "`IU` | SDDL_INTERACTIVE |
> Interactively logged-on user."; "`AN` | SDDL_ANONYMOUS"; "`WD` | SDDL_EVERYONE";
> "`BA` | SDDL_BUILTIN_ADMINISTRATORS".

`ConvertStringSecurityDescriptorToSecurityDescriptorA` (https://learn.microsoft.com/en-us/windows/win32/api/sddl/nf-sddl-convertstringsecuritydescriptortosecuritydescriptora)

> "`StringSDRevision`: Specifies the revision level of the
> StringSecurityDescriptor string. Currently this value must be SDDL_REVISION_1."
> "`SecurityDescriptor`: A pointer to a variable that receives a pointer to the
> converted security descriptor. The returned security descriptor is
> self-relative. To free the returned buffer, call the LocalFree function. To
> convert the security descriptor to an absolute security descriptor, use the
> MakeAbsoluteSD function."

`Mandatory Integrity Control` (https://learn.microsoft.com/en-us/windows/win32/secauthz/mandatory-integrity-control)

> "For example, a principal with a low integrity level cannot write to an object
> with a medium integrity level, even if that object's DACL allows write access
> to the principal." ... "Objects that lack an integrity label are treated as
> medium by the operating system" ... "By default, the system creates every
> object with an access mask of SYSTEM_MANDATORY_LABEL_NO_WRITE_UP."
> "The integrity SID for a securable object is stored in its system access
> control list (SACL)."

`SACL Access Right` (https://learn.microsoft.com/en-us/windows/win32/secauthz/sacl-access-right)

> "The ACCESS_SYSTEM_SECURITY access right controls the ability to get or set
> the SACL in an object's security descriptor. The system grants this access
> right only if the SE_SECURITY_NAME privilege is enabled in the access token of
> the requesting thread."

`Named Pipe Reject Remote Clients` (HLK test reference) (https://learn.microsoft.com/en-us/windows-hardware/test/hlk/testref/e3bcbd3f-3e9c-484a-a587-3c081cb28f7a)

> "This automated test verifies that the GetNamedPipeInfo function returns the
> correct option when the named pipe is created with the
> PIPE_REJECT_REMOTE_CLIENTS option."

`Getting the Logon SID in C++` (archived) (https://learn.microsoft.com/en-us/previous-versions/aa446670(v=vs.85))

> "A logon security identifier (SID) identifies the logon session associated
> with an access token." ... the sample calls `GetTokenInformation` with
> `TokenGroups` and selects the entry whose attributes contain
> `SE_GROUP_LOGON_ID`.

### Rust crates

`tokio::net::windows::named_pipe::ServerOptions` (https://docs.rs/tokio/latest/tokio/net/windows/named_pipe/struct.ServerOptions.html)

> "Indicates whether this server can accept remote clients or not. Remote
> clients are disabled by default. This corresponds to setting
> PIPE_REJECT_REMOTE_CLIENTS."

> `create_with_security_attributes_raw`: "This is the same as `create` except
> that it supports providing the raw pointer to a structure of
> SECURITY_ATTRIBUTES which will be passed as the `lpSecurityAttributes`
> argument to `CreateFile`." (the parameter name is right, the API name in the
> sentence is a doc bug, see Contradictions)

tokio source, `tokio/src/net/windows/named_pipe.rs` (https://raw.githubusercontent.com/tokio-rs/tokio/master/tokio/src/net/windows/named_pipe.rs)

> `pub fn new() -> ServerOptions { ServerOptions { ... pipe_mode: PipeMode::Byte,
> reject_remote_clients: true, max_instances: windows_sys::PIPE_UNLIMITED_INSTANCES, ... } }`

> `pub unsafe fn create_with_security_attributes_raw(&self, addr: impl
> AsRef<OsStr>, attrs: *mut c_void) -> io::Result<NamedPipeServer>` ... `let h =
> unsafe { windows_sys::CreateNamedPipeW(addr.as_ptr(), open_mode, pipe_mode,
> self.max_instances, self.out_buffer_size, self.in_buffer_size,
> self.default_timeout, attrs as *mut _, ) };`

> `if self.reject_remote_clients { mode |= windows_sys::PIPE_REJECT_REMOTE_CLIENTS; } else { mode |= windows_sys::PIPE_ACCEPT_REMOTE_CLIENTS; }`

`interprocess::os::windows::named_pipe::PipeListenerOptions` (https://docs.rs/interprocess/latest/x86_64-pc-windows-msvc/interprocess/os/windows/named_pipe/struct.PipeListenerOptions.html)

> Fields include `pub accept_remote: bool` ("Enables remote machines to connect
> to the named pipe over the network.") and `pub security_descriptor:
> Option<SecurityDescriptor>` ("The security descriptor to create the named pipe
> server with."), plus `create_tokio_duplex` and friends behind the `tokio`
> feature.

interprocess source, `src/os/windows/named_pipe/listener/options.rs` (https://raw.githubusercontent.com/kotauskas/interprocess/main/src/os/windows/named_pipe/listener/options.rs, mirrored at docs.rs source)

> `Self { path: ..., mode: PipeMode::Bytes, nonblocking: false, instance_limit:
> None, write_through: false, accept_remote: false, ..., security_descriptor:
> None, inheritable: false }`

interprocess source, `src/os/windows/named_pipe/listener/create_instance.rs` (https://raw.githubusercontent.com/kotauskas/interprocess/main/src/os/windows/named_pipe/listener/create_instance.rs)

> `let sa = create_security_attributes(self.security_descriptor.as_ref().map(|sd| sd.borrow()), self.inheritable);` ... `CreateNamedPipeW((*self.path).as_ptr(), open_mode, pipe_mode, max_instances, ..., ref2ptr(&sa).cast_mut().cast())`

> `fn pipe_mode(&self, recv_mode: Option<PipeMode>, nonblocking: bool) -> u32 { ... if !self.accept_remote { pipe_mode |= PIPE_REJECT_REMOTE_CLIENTS; } pipe_mode }`

> `if first { open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE; }`

`interprocess::os::windows::local_socket::ListenerOptionsExt` (https://docs.rs/interprocess/latest/x86_64-pc-windows-msvc/interprocess/local_socket/struct.ListenerOptions.html)

> "`method.security_descriptor` Available on Windows only. Sets the security
> descriptor that will control access to the underlying named pipe."

interprocess source, `src/os/windows/security_descriptor/owned.rs`

> `pub fn new() -> io::Result<Self>` ... `pub fn deserialize(sdsf: &U16CStr) ->
> io::Result<Self>` ("Deserializes a security descriptor from the security
> descriptor string format") ... `unsafe fn from_owned` with safety notes: "The
> security descriptor must be absolute, not self-relative. The security
> descriptor must own all of its contents."

`windows_sys::Win32::System::Pipes` (https://docs.rs/windows-sys/latest/windows_sys/Win32/System/Pipes/index.html) and `windows_sys::Win32::Security`

> `ConvertStringSecurityDescriptorToSecurityDescriptorW(stringsecuritydescriptor:
> PCWSTR, stringsdrevision: u32, securitydescriptor: *mut PSECURITY_DESCRIPTOR,
> securitydescriptorsize: *mut u32) -> BOOL` in
> `windows_sys::Win32::Security::Authorization`.

> `#[repr(C)] pub struct SECURITY_ATTRIBUTES { pub nLength: u32, pub
> lpSecurityDescriptor: *mut c_void, pub bInheritHandle: BOOL }` in
> `windows_sys::Win32::Security`.

> `GetNamedPipeInfo`, `GetNamedPipeServerProcessId`, `GetNamedPipeClientSessionId`
> all exist as `pub unsafe extern "system" fn` in `windows_sys::Win32::System::Pipes`.

`windows-link` (https://docs.rs/windows-link/latest/windows_link/)

> "The windows-link crate provides the `link` macro that simplifies linking. The
> `link` macro uses `raw-dylib` and thus does not require import lib files."

Secondary (used only as pointers, not as evidence for the decisive claims):
Stack Overflow "Named Pipe Security and Access Rights"-adjacent answers,
`https://stackoverflow.com/questions/9589141/low-integrity-to-medium-high-integrity-pipe-security-descriptor`
for the integrity-label reasoning, and
`https://github.com/openai/codex/blob/2230d644/codex-rs/windows-sandbox-rs/src/elevated/runner_pipe.rs`
as an example of a Rust pipe DACL helper.

## Findings

1. **Claim:** The Win32 mechanism is `CreateNamedPipeW` with a non-NULL
   `lpSecurityAttributes` pointing at a `SECURITY_ATTRIBUTES` whose
   `lpSecurityDescriptor` is the ACL that restricts clients, and the
   `dwPipeMode` bit `PIPE_REJECT_REMOTE_CLIENTS` for the network boundary.
   **Sources:** CreateNamedPipeW; Named Pipe Security and Access Rights.
   **Support:** direct evidence (quoted doc text). **Confidence:** high.

2. **Claim:** Never pass `NULL` for `lpSecurityAttributes` for this daemon. The
   default descriptor grants read access to Everyone and to the anonymous
   account, plus full control to LocalSystem, administrators, and the creator
   owner. **Sources:** CreateNamedPipeW; Named Pipe Security and Access Rights.
   **Support:** direct evidence. **Confidence:** high.

3. **Claim:** The concrete descriptor for the intended-user-only case is the
   SDDL string

   ```
   D:P(D;;GA;;;NU)(A;;GA;;;<daemon-user-SID>)(A;;GA;;;SY)
   ```

   built at startup with `<daemon-user-SID>` read from the daemon's own token,
   and converted with `ConvertStringSecurityDescriptorToSecurityDescriptorW`
   using `SDDL_REVISION_1`. Justification per part: `D:P` sets
   `SE_DACL_PROTECTED` so inherited ACEs cannot widen it (Security Descriptor
   String Format); the first ACE is a deny for `NU`, the "Network logon user"
   group added to tokens "when it was logged on across a network" (SID
   Strings), which is the documented alternative to the reject flag ("deny
   access to NT AUTHORITY\NETWORK", Named Pipes); `A;;GA;;;<SID>` is an
   access-allowed ACE granting `GENERIC_ALL` (`GA` = GENERIC_ALL, ACE Strings)
   to the daemon's own account, which also gives the daemon the
   `FILE_CREATE_PIPE_INSTANCE` right it needs for later instances because
   `FILE_GENERIC_WRITE` contains `FILE_APPEND_DATA`, which shares the value
   `FILE_CREATE_PIPE_INSTANCE` (Named Pipe Security and Access Rights);
   `SY` is LocalSystem, included so system tooling and reset paths still work.
   **Sources:** as listed. **Support:** direct evidence for each documented
   rule; the assembled string itself is researcher construction
   (interpretation). **Confidence:** high on syntax, medium on the specific
   mask choice (the alternative fine-grained mask is `GR`+`GW` for the user,
   which the docs suggest instead of GENERIC_WRITE when instance creation is
   not wanted; here it is wanted).

4. **Claim:** "Intended users" for a per-user personal daemon means exactly one
   principal, the account that owns the daemon process, resolved at runtime
   from its token (`OpenProcessToken` + `GetTokenInformation(TokenUser)` and
   `ConvertSidToStringSidW`, or `AllocateAndInitializeSid` plus
   `SetEntriesInAcl`). It is not a group, not `IU`, not `WD`. This matches the
   Unix side of the same map constraint, where the socket is owned by a
   configured uid:gid with mode 0700 or 770, and it means any process running
   as that account can connect. If the intent were instead "this logon session
   only", the documented mechanism is the logon SID in the DACL, which is
   stricter than the Unix equivalent because it excludes the same account in a
   different session, and it changes on every logon (the daemon would have to
   be restarted per logon to keep the descriptor current).
   **Sources:** Named Pipe Security and Access Rights (logon SID sentence);
   Getting the Logon SID in C++; `docs/tasks/metis-foundations/map.md`
   (decisions: local IPC authenticates with the host ACL, mode 700 or 770 on
   Unix, security descriptor on Windows). **Support:** direct evidence for the
   mechanisms; the mapping of "intended users" to the single owning account is
   researcher inference. **Confidence:** high on mechanisms, medium on the
   naming decision, which is a product choice for the parent.

5. **Claim:** `PIPE_REJECT_REMOTE_CLIENTS` is the supported mechanism and the
   ACL alone is not sufficient, because a pipe is reachable over SMB whenever
   the server service runs and a remote caller using the same credentials
   produces a token with the same user SID. **Sources:** CreateNamedPipeW
   ("Connections from remote clients are automatically rejected"); Named Pipes
   ("If the server service is running, all named pipes are accessible
   remotely. If you intend to use a named pipe locally only, deny access to NT
   AUTHORITY\NETWORK or switch to local RPC."); Named Pipe Security and Access
   Rights ("To prevent remote users or users on a different terminal services
   session from accessing a named pipe, use the logon SID on the DACL for the
   pipe."). **Support:** direct evidence that the docs treat "local only" as a
   separate concern from the DACL and recommend two different remote-blocking
   mechanisms; the SID-identity mechanism behind it is researcher
   interpretation (the docs do not spell out the token details of an SMB
   connection to a named pipe). **Confidence:** high that the flag is required,
   medium on the mechanism explanation.

6. **Claim:** How a remote attempt fails is only partly documented. The
   documented statements are that such connections are "automatically
   rejected", and that `GetNamedPipeInfo` reports the option, which is what
   the only Microsoft test in this area asserts. The client-visible Win32 error
   for a rejected remote connect is not specified in the sources I fetched.
   **Sources:** CreateNamedPipeW; Named Pipe Reject Remote Clients (HLK).
   **Support:** direct evidence for "automatically rejected"; the inference
   (labelled as such) is that rejection happens in the pipe file system or SMB
   layer before an instance-level access check, so the remote caller most
   likely sees a name-not-found style failure instead of `ERROR_ACCESS_DENIED`,
   and the daemon never observes a connection event. The belt-and-braces
   `NU` deny ACE would turn a remote attempt into a documented access-check
   denial (`ERROR_ACCESS_DENIED`) if the request ever reaches the ACL.
   **Confidence:** medium for the inference, low for any specific error code.

7. **Claim:** `tokio::net::windows::named_pipe` passes `SECURITY_ATTRIBUTES`
   and rejects remote clients, so this crate can back both the daemon listener
   and the client connector without an extra IPC crate.
   `ServerOptions::reject_remote_clients(bool)` exists and defaults to `true`
   (source: `reject_remote_clients: true` in `ServerOptions::new()`).
   `ServerOptions::create_with_security_attributes_raw(path, attrs: *mut c_void)`
   is an `unsafe fn` whose pointer becomes `lpSecurityAttributes` of the
   `CreateNamedPipeW` call. `ServerOptions::first_pipe_instance(true)` maps to
   `FILE_FLAG_FIRST_PIPE_INSTANCE`, and the default pipe mode is
   `PipeMode::Byte`, which matches the duplex byte-mode requirement in the map
   and the rough plan. `NamedPipeServer` implements `AsyncRead`/`AsyncWrite`,
   so hyper's HTTP/1.1 handshake and upgrade can run over the pipe directly.
   **Sources:** ServerOptions docs; tokio source; NamedPipeServer docs.
   **Support:** direct evidence (crate source read). **Confidence:** high.
   Version read: tokio 1.x (docs.rs `latest` showed 1.53.2).

8. **Claim:** `interprocess` 2.4.5 also supports all of this, on two levels.
   The cross-platform `local_socket::ListenerOptions` has a Windows-only
   extension method `security_descriptor(SecurityDescriptor)`. The
   Windows-specific `PipeListenerOptions` exposes `security_descriptor` and
   `accept_remote`, defaults `accept_remote` to `false`, sets
   `PIPE_REJECT_REMOTE_CLIENTS` when `accept_remote` is false, builds
   `SECURITY_ATTRIBUTES` from the descriptor, and passes it to
   `CreateNamedPipeW`; the first instance also gets
   `FILE_FLAG_FIRST_PIPE_INSTANCE`. Two frictions for this project: SDDL entry
   is `SecurityDescriptor::deserialize(&U16CStr)`, which requires the
   `widestring` crate in the call site, and the wrapper demands an absolute,
   self-owned descriptor while the Windows SDDL conversion API returns a
   self-relative descriptor, so mixing `interprocess::SecurityDescriptor` with
   `ConvertStringSecurityDescriptorToSecurityDescriptorW` needs
   `MakeAbsoluteSD` or a copy. **Sources:** interprocess docs and source (see
   above). **Support:** direct evidence (crate source read). **Confidence:**
   high.

9. **Claim:** The raw Win32 fallback exists and is small, so the transport
   adapter does not depend on any wrapper crate. `windows-sys` provides
   `Win32::System::Pipes::{CreateNamedPipeW, GetNamedPipeInfo,
   GetNamedPipeServerProcessId, GetNamedPipeClientSessionId,
   PIPE_REJECT_REMOTE_CLIENTS}`, `Win32::Security::{SECURITY_ATTRIBUTES,
   OpenProcessToken, GetTokenInformation, ...}` and
   `Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW`.
   On linking: `windows-sys` 0.61 is generated against `windows-link`, whose
   documented behavior is that "The link macro uses raw-dylib and thus does not
   require import lib files". Only Windows system DLLs (kernel32, advapi32)
   are involved and none of them is a third-party shared library, so the
   "no dynamically linked system libraries" constraint in
   `docs/tasks/metis-foundations/map.md` (which is stated next to the rustls
   decision) is satisfied in spirit by `windows-sys`; researcher inference: it
   cannot literally mean "no Win32 DLL", because Win32 APIs are always imported
   from system DLLs. **Sources:** docs.rs `windows_sys` pages; windows-link
   docs; map. **Support:** direct evidence for API paths and the raw-dylib
   statement; the constraint interpretation is researcher inference.
   **Confidence:** high on API paths, medium on the feature flag names
   (`Win32_Security`, `Win32_Security_Authorization`, `Win32_System_Pipes`,
   `Win32_Foundation` is the expected shape but was not confirmed against the
   feature list; verify by compiling).

10. **Claim:** The listener adapter shape (not compiled or run here; Linux
    host):

    ```rust
    // windows transport module, listener side
    let sddl = format!("D:P(D;;GA;;;NU)(A;;GA;;;{user_sid})(A;;GA;;;SY)\0"); // UTF-16 in practice
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let mut sd_len = 0u32;
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_wide.as_ptr(), SDDL_REVISION_1, &mut sd, &mut sd_len,
        );
    }
    let mut attrs = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            // .reject_remote_clients(true) is already the default, keep it explicit
            .create_with_security_attributes_raw(
                r"\\.\pipe\metis",
                (&mut attrs as *mut SECURITY_ATTRIBUTES).cast(),
            )?
    };
    // CreateNamedPipeW copies what it needs; release the SD with LocalFree afterwards.
    ```

    Then the accept loop creates the next instance before handing the connected
    one to a task (tokio's documented pattern for the
    `io::ErrorKind::NotFound` race), and each connection is served with the
    same generic HTTP handler as the Unix socket. **Sources:** CreateNamedPipeW
    (`lpSecurityAttributes` is consumed during creation); tokio
    NamedPipeServer docs (listen-loop pattern); ConvertStringSecurityDescriptor
    docs (LocalFree). **Support:** interpretation plus direct evidence for each
    call. **Confidence:** medium (unverified by compilation, and the
    LocalFree timing is a standard pattern rather than a stated rule).

11. **Claim:** Session-boundary caveats that the intended-user descriptor does
    not cover.
    - Another logon session of the same user (console plus RDP, fast user
      switching, a scheduled task running as that user) can connect, because
      the DACL only knows the user SID. The documented remedy is the logon SID
      in the DACL. **Sources:** Named Pipe Security and Access Rights;
      Getting the Logon SID in C++. **Confidence:** high.
    - Low-integrity same-user processes (sandboxed renderers, AppContainer
      children): Mandatory Integrity Control is a separate mechanism from the
      DACL and lives in the SACL, and unlabeled objects are treated as medium
      with `NO_WRITE_UP`. I found no primary statement that the named pipe file
      system applies MIC to pipe objects, so the safe reading is "unverified,
      do not rely on it". **Sources:** Mandatory Integrity Control (direct
      evidence for the general rule); the secondary Stack Overflow answer
      (https://stackoverflow.com/questions/9589141/...) for the pipe-specific
      reading. **Confidence:** medium that MIC would block low-integrity writes
      to an unlabeled pipe, low that the DACL alone does anything about it.
    - Setting a mandatory label on the pipe requires writing the SACL, which
      requires `ACCESS_SYSTEM_SECURITY` and the `SE_SECURITY_NAME` privilege
      (SeSecurityPrivilege). An unelevated per-user daemon does not hold it, so
      this hardening is not available by default; the alternative is to check
      the client after `ImpersonateNamedPipeClient` via
      `GetTokenInformation(TokenIntegrityLevel)`. **Sources:** SACL Access
      Right; CreateNamedPipeW (`ACCESS_SYSTEM_SECURITY` mode);
      Impersonating a Named Pipe Client. **Confidence:** high on the privilege
      requirement, high that the impersonation check is available.
    - The `\\?\pipe\` path form: I found no primary documentation for it in the
      pipe docs. Documented forms are `\\.\pipe\name` (server and local client)
      and `\\ServerName\pipe\name` (client, "." for the local machine); the only
      documented alternative namespace is the app-container form
      `\\.\pipe\LOCAL\`. Researcher inference: the DACL lives on the pipe object,
      so any spelling that resolves to the same object gets the same access
      check, which means `\\?\pipe\` is not a separate security boundary. Treat
      the exact behavior of that spelling as unverified and do not test against
      it. **Sources:** Pipe Names; CreateNamedPipeW (app-container note).
      **Confidence:** high for the documented forms, medium for the inference.

12. **Claim:** Client-side consequences. The connector opens
    `\\.\pipe\<name>` (never a real server name, which would be a remote
    attempt and would be rejected by the flag), with `GENERIC_READ |
    GENERIC_WRITE`, and maps OS errors to typed errors without falling back to
    TCP: `ERROR_ACCESS_DENIED` means the pipe ACL did not admit this account
    (the access check is performed before the client is granted access);
    `ERROR_FILE_NOT_FOUND` means the daemon is not listening or the name is not
    reachable at all; `ERROR_PIPE_BUSY` means every instance is in use;
    `ERROR_PIPE_NOT_CONNECTED`/`ERROR_BROKEN_PIPE` means the daemon side went
    away or the connection dropped. On the daemon side, a connection can be
    re-authenticated by `ImpersonateNamedPipeClient` followed by
    `GetTokenInformation(TokenUser)`, and locality can be inspected with
    `GetNamedPipeClientSessionId` (and `GetNamedPipeClientProcessId`); the
    client can inspect the server with `GetNamedPipeServerProcessId`, although
    there is no single call that proves "this is the metis daemon", so a client
    running as the same user as an attacker is not protected against a squatter
    that started first. The daemon should create its first instance with
    `FILE_FLAG_FIRST_PIPE_INSTANCE` (tokio `first_pipe_instance(true)`) so that
    it fails loudly instead of coexisting with a squatter. **Sources:**
    Named Pipe Security and Access Rights (client access check); Named Pipe
    Client (ERROR_PIPE_BUSY); CreateNamedPipeW
    (`FILE_FLAG_FIRST_PIPE_INSTANCE`); Impersonating a Named Pipe Client;
    tokio and windows-sys API pages. **Support:** direct evidence for each API
    and error; the error-mapping policy is researcher recommendation.
    **Confidence:** high on the API facts, medium on the recommended mapping
    (error codes for the ACL and remote cases should be confirmed on Windows).

## Contradictions

- **tokio documentation versus tokio source.** The doc comment for
  `create_with_security_attributes_raw` says the pointer "will be passed as the
  `lpSecurityAttributes` argument to `CreateFile`", but the function body calls
  `windows_sys::CreateNamedPipeW(..., attrs as *mut _)`. The source is
  authoritative for behavior; this is a documentation bug, recorded here so the
  implementer does not conclude the API is client-side only.
- **Common secondary guidance versus Microsoft guidance.** Several secondary
  sources (and the popular "authenticate by DACL" pattern) present an ACL keyed
  on the user as making a pipe local-only. Microsoft's own pages treat local-only
  as a separate switch (`PIPE_REJECT_REMOTE_CLIENTS`) or a separate ACE (deny
  `NU`), and the ACL page recommends the logon SID for session and remote
  exclusion. The Microsoft pages are treated as authoritative here.
- **`interprocess` behaves as if local-only and remote-accept are distinct.**
  Its `write_through` documentation says setting it on a local-only pipe panics
  under debug assertions, which is consistent with the reject-remote flag being
  the thing that defines "local-only". No contradiction with the findings; noted
  because it confirms the library models the same boundary.

## Missing evidence

- The exact Win32 error a remote client observes for a pipe created with
  `PIPE_REJECT_REMOTE_CLIENTS`. Not documented in the pages fetched; needs a
  two-machine or SMB-loopback experiment.
- Whether the named pipe file system enforces Mandatory Integrity Control on
  pipe objects, that is, whether a low-integrity same-user process is actually
  denied write access to an unlabeled pipe.
- The exact `windows-sys` feature flag names for the items listed in finding 9,
  and whether `SDDL_REVISION_1` and `PSECURITY_DESCRIPTOR` are re-exported at
  the paths assumed. Verify by compiling for `x86_64-pc-windows-msvc`.
- Any primary source describing a `\\?\pipe\` namespace, and whether
  `\\.\pipe\LOCAL\` is meaningful outside app containers.
- No Windows runner was available, so nothing above was executed or compiled;
  in particular `PIPE_REJECT_REMOTE_CLIENTS` is not verified to be present in
  the `windows-sys` version that tokio pulls in (the interprocess source, which
  parses against a recent `windows-sys`, does use
  `windows_sys::Win32::System::Pipes::PIPE_REJECT_REMOTE_CLIENTS`, which is
  indirect but strong evidence that it exists).

## Recommendation and decision input

1. Windows transport: `tokio::net::windows::named_pipe` in both crates. The
   daemon listener uses `ServerOptions` with `first_pipe_instance(true)`,
   keeps `reject_remote_clients(true)` (explicitly, even though it is the
   default), and passes the descriptor through
   `create_with_security_attributes_raw`. The connector uses `ClientOptions`.
   Both operate on byte mode duplex pipes and speak the same HTTP and WebSocket
   protocol as the Unix socket path, which satisfies "one endpoint
   implementation shared across local and remote transports".
2. Dependency footprint: add `windows-sys` (target-gated) with the security and
   pipes features and use it for `SECURITY_ATTRIBUTES`,
   `ConvertStringSecurityDescriptorToSecurityDescriptorW`, and the token
   queries. This avoids adding `interprocess` plus `widestring`, and keeps one
   `AsyncRead + AsyncWrite` type for hyper. Choose `interprocess` instead only
   if the cross-platform `local_socket` name abstraction is wanted more than the
   smaller dependency tree; its Windows security API is complete for this task
   either way.
3. Descriptor: `D:P(D;;GA;;;NU)(A;;GA;;;<daemon-user-SID>)(A;;GA;;;SY)` built
   at startup from the daemon token, with the pipe name derived from the
   endpoint configuration (platform default plus environment override). The
   override should be validated to start with `\\.\pipe\` so that an operator
   cannot quietly point the daemon at a `\\server\pipe\name` path.
4. Both halves of the map constraint are implemented: the descriptor restricts
   the pipe to the intended user (and SYSTEM), and
   `PIPE_REJECT_REMOTE_CLIENTS` makes it local-only. Document the accepted
   residual: any process running as that same account on the same machine,
   including another session, can connect, exactly as mode 0700 behaves on
   Unix.
5. Optional hardening, only if the threat model includes same-user
   low-integrity code: check the impersonated client's
   `TokenIntegrityLevel` (and optionally its session id) after accept, because
   setting a mandatory label on the pipe needs SeSecurityPrivilege, which the
   unelevated daemon does not have.

## Impact on dependent tasks

- The `metis-daemon` Windows listener task can implement the accept loop and
  the descriptor exactly as above and needs no `interprocess` decision blocking
  it; the fallback to raw Win32 is not required at all with tokio, because
  tokio's raw-attributes entry point covers the descriptor.
- The `metis-foundations` connector task gets a concrete client path form and an
  error-mapping policy, plus the note that no fallback to TCP should happen on
  local IPC failures.
- The endpoint configuration decision (platform default plus environment
  variable override) gains a validation requirement: the Windows default must
  be a `\\.\pipe\` name, and the override should be checked for the same prefix.
- The CI matrix task gains a Windows runner requirement: the only automatable
  assertions that need Windows are the descriptor inspection, the
  `GetNamedPipeInfo` mode check, the same/different user connect tests, and
  (optionally, two hosts) the remote rejection probe.
- If `interprocess` is chosen instead, the task should also budget for
  `widestring` at the SDDL call site and for the absolute-versus-self-relative
  descriptor difference.

## Minimal verification plan

No Windows runner was available for this task. The smallest set of checks that
would close the gaps, in order:

1. On a Windows runner (or a Windows host, documented as a manual check),
   start the listener and call `GetNamedPipeInfo`, asserting the reported mode
   includes the reject-remote option. This is the same assertion Microsoft's own
   HLK test makes.
2. Read back the descriptor with `GetSecurityInfo` (`SE_KERNEL_OBJECT`) and
   assert the DACL contains exactly the intended ACEs and no ACE for `WD`, `AN`,
   or `BA`.
3. A client running as the daemon user connects and completes one HTTP request;
   a client running as a different local user fails. Record the exact error for
   that second case and assert it is not treated as "daemon not running".
4. Record the `GetLastError` a remote attempt produces (from a second machine,
   or from the same machine using `\\<own-hostname>\pipe\<name>`): either from
   `CreateFile` directly or by catching the .NET `NamedPipeClientStream`
   exception. This is the only way to replace finding 6's inference with a fact.
5. Only if the low-integrity threat is in scope: run a low-integrity client and
   record whether it can connect and write, then decide between the
   impersonation integrity check and requiring an elevated daemon.

## Unresolved questions

- Does the product want "the user" or "the current logon session" as the pipe
  principal? The logon SID gives strict session isolation at the cost of
  surviving re-logon, which matters for a long-running personal daemon.
- Should SYSTEM keep full control of the pipe? It is conventional and useful for
  inspection tools, but it is an extra principal in the descriptor.
- Is a squatter-resistant client check (server process image verification via
  `GetNamedPipeServerProcessId`) required for the walking skeleton, or is it
  deferred?
- Do we need the `NU` deny ACE at all, given that the reject-remote flag already
  covers the supported targets (Vista and later)? It costs one ACE and turns an
  otherwise unspecified failure into a documented denial.
