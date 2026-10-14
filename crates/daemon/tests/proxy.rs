//! Reverse-proxy mode over a real listener.
//!
//! The rule itself is unit-tested in `crates/api-contracts/src/proxy.rs`. What
//! no unit test can show is asserted here: that the peer of the connection
//! actually reaches a handler over the TCP front-end, so the rule is reachable
//! end to end, and that the local socket, which has no peer, forwards nothing.
//!
//! The service served here is this binary's own: an integration test binary
//! carries the setup it drives, the convention the daemon's other test binaries
//! set. It answers echo and records the leg the call arrived on, so what is
//! asserted is what a caller's call produced and not what a function returns.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};

use api_contracts::connect::metis::v1::{DaemonService, DaemonServiceClient, DaemonServiceExt};
use api_contracts::proto::metis::v1::{
    CancelRequest, CancelResponse, EchoRequest, EchoResponse, Event, PairRequest, PairResponse,
    StreamEventsRequest,
};
use api_contracts::proxy::{
    ClientAddress, ClientScheme, TrustedProxies, client_address, client_scheme,
};
use api_contracts::transport::{serve_tcp_listener_service, tcp_transport};
use connectrpc::client::{ClientConfig, SharedHttp2Connection};
use connectrpc::{
    ConnectError, ConnectRpcService, ErrorCode, RequestContext, Response, Router, ServiceRequest,
    ServiceResult, ServiceStream,
};

/// The address the tests name as the proxy, which is the peer a loopback
/// listener sees.
const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// The client address the tests' proxy forwards, from the documentation range so
/// it can never be the peer of a loopback connection.
const FORWARDED: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));

/// What one served call observed about where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Leg {
    /// The peer of the connection, as `RequestContext::peer_addr` reports it.
    peer: Option<SocketAddr>,
    /// The address the proxy rule attributes the call to.
    client: Option<ClientAddress>,
    /// The scheme the rule reports for the client leg.
    scheme: ClientScheme,
}

/// The service the tests serve: it records the leg of every echo call and
/// answers it, so the front-end's stamp is observable through a real call.
struct Observer {
    trusted: TrustedProxies,
    seen: Arc<Mutex<Option<Leg>>>,
}

impl Observer {
    fn new(trusted: TrustedProxies, seen: Arc<Mutex<Option<Leg>>>) -> Self {
        Self { trusted, seen }
    }
}

// The generated trait uses an explicit RPITIT signature; an `async fn` impl
// refines it, which the compiler flags with `refining_impl_trait`.
#[allow(refining_impl_trait)]
impl DaemonService for Observer {
    /// Answer the echo and record the leg it arrived on, which is the only
    /// handler these tests drive.
    async fn echo(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, EchoRequest>,
    ) -> Result<Response<EchoResponse>, ConnectError> {
        let peer = ctx.peer_addr();
        let address = peer.map(|addr| addr.ip());
        let leg = Leg {
            peer,
            client: client_address(address, &self.trusted, ctx.headers()),
            scheme: client_scheme(address, &self.trusted, ctx.headers()),
        };
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(leg);
        Response::ok(EchoResponse {
            message: request.message.to_owned(),
            ..Default::default()
        })
    }

    /// Pairing, the stream, and cancel belong to the daemon's own service; this
    /// recorder covers the whole trait so the router registers, and answers
    /// unimplemented for the calls it does not drive.
    async fn pair(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, PairRequest>,
    ) -> Result<Response<PairResponse>, ConnectError> {
        Err(unimplemented("pair"))
    }

    async fn stream_events(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, StreamEventsRequest>,
    ) -> ServiceResult<ServiceStream<Event>> {
        Err(unimplemented("stream_events"))
    }

    async fn cancel(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, CancelRequest>,
    ) -> ServiceResult<CancelResponse> {
        Err(unimplemented("cancel"))
    }
}

/// The refusal for a call this recorder does not drive.
fn unimplemented(procedure: &str) -> ConnectError {
    ConnectError::new(
        ErrorCode::Unimplemented,
        format!("this recorder serves echo only, not {procedure}"),
    )
}

/// The service a listener serves for `observer`.
fn service(observer: Observer) -> ConnectRpcService<Router> {
    ConnectRpcService::from_arc(Arc::new(Arc::new(observer).register(Router::new())))
}

/// Serve `observer` on a loopback TCP listener the kernel binds, and report the
/// address it chose.
async fn serve_over_tcp(observer: Observer) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind a loopback port");
    let addr = listener.local_addr().expect("read the bound address");
    let service = service(observer);
    tokio::spawn(async move {
        if let Err(err) = serve_tcp_listener_service(listener, service).await {
            eprintln!("proxy test listener error: {err}");
        }
    });
    addr
}

/// The generated client over TCP, declaring the forwarded headers a proxy would
/// add when `forwarded` names the client the proxy reports.
fn tcp_client(
    addr: SocketAddr,
    forwarded: Option<IpAddr>,
) -> DaemonServiceClient<SharedHttp2Connection> {
    let transport = tcp_transport(addr).shared(64);
    let mut config = ClientConfig::new(api_contracts::transport::remote_authority(addr));
    if let Some(client) = forwarded {
        config = config
            .with_default_header("x-forwarded-for", client.to_string())
            .with_default_header("x-forwarded-proto", "https");
    }
    DaemonServiceClient::new(transport, config)
}

/// Call echo through `daemon`, which is the one call these tests need.
async fn echo(daemon: &DaemonServiceClient<SharedHttp2Connection>) {
    daemon
        .echo(EchoRequest {
            message: "hello".to_owned(),
            ..Default::default()
        })
        .await
        .expect("the call is served");
}

/// The leg the last served call reported.
fn observed(seen: &Arc<Mutex<Option<Leg>>>) -> Leg {
    seen.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .expect("a call was served")
}

/// The peer of the connection reaches the handler over TCP, which is what makes
/// the proxy rule reachable end to end, and the rule names it as the client when
/// nobody is trusted.
#[tokio::test]
async fn the_peer_that_opened_the_connection_reaches_the_handler() {
    let seen = Arc::new(Mutex::new(None));
    let trusted = TrustedProxies::parse("").expect("nothing is named");
    let addr = serve_over_tcp(Observer::new(trusted, Arc::clone(&seen))).await;

    echo(&tcp_client(addr, None)).await;

    let leg = observed(&seen);
    let peer = leg.peer.expect("a TCP connection has a peer");
    assert_eq!(peer.ip(), LOOPBACK, "the peer is the caller's address");
    assert_ne!(
        peer.port(),
        0,
        "the peer carries the port it connected from"
    );
    assert_eq!(
        leg.client,
        Some(ClientAddress::Peer(LOOPBACK)),
        "an unnamed peer is the client"
    );
    assert_eq!(leg.scheme, ClientScheme::Plaintext);
}

/// The local socket has no peer, so nothing is forwarded on its behalf even when
/// the caller sends forwarded headers.
#[cfg(unix)]
#[tokio::test]
async fn the_local_socket_has_no_peer_and_forwards_nothing() {
    use api_contracts::transport::{local_authority, serve_uds_service, uds_transport};
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;

    let seen = Arc::new(Mutex::new(None));
    let trusted = TrustedProxies::parse("127.0.0.1").expect("the list parses");
    let directory = std::env::temp_dir().join(format!("metis-proxy-uds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create the socket directory");
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        .expect("restrict the socket directory");
    let path: PathBuf = directory.join("daemon.sock");

    let service = service(Observer::new(trusted, Arc::clone(&seen)));
    let served = path.clone();
    tokio::spawn(async move {
        if let Err(err) = serve_uds_service(&served, service).await {
            eprintln!("proxy test socket error: {err}");
        }
    });
    assert!(
        client::skeleton::wait_for_socket(&path).await,
        "the listener binds its socket"
    );

    let transport = uds_transport(path.clone()).shared(64);
    let config = ClientConfig::new(local_authority())
        .with_default_header("x-forwarded-for", FORWARDED.to_string())
        .with_default_header("x-forwarded-proto", "https");
    let daemon = DaemonServiceClient::new(transport, config);
    echo(&daemon).await;

    let leg = observed(&seen);
    assert_eq!(leg.peer, None, "a local connection has no peer");
    assert_eq!(leg.client, None, "nothing is forwarded for a local caller");
    assert_eq!(leg.scheme, ClientScheme::Plaintext);

    let _ = std::fs::remove_dir_all(&directory);
}

/// The same forwarded header is honoured when the peer that sent it was named
/// and ignored when it was not, over the real listener both clients reach.
#[tokio::test]
async fn forwarded_information_is_accepted_from_a_named_proxy_and_ignored_from_an_unnamed_peer() {
    let named_seen = Arc::new(Mutex::new(None));
    let named = TrustedProxies::parse(&LOOPBACK.to_string()).expect("the list parses");
    let named_addr = serve_over_tcp(Observer::new(named, Arc::clone(&named_seen))).await;
    echo(&tcp_client(named_addr, Some(FORWARDED))).await;

    let leg = observed(&named_seen);
    assert_eq!(
        leg.client,
        Some(ClientAddress::Forwarded(FORWARDED)),
        "a named proxy's forwarded address is the client"
    );
    assert_eq!(
        leg.scheme,
        ClientScheme::Encrypted,
        "a named proxy saying it terminated TLS is believed"
    );

    let unnamed_seen = Arc::new(Mutex::new(None));
    let unnamed = TrustedProxies::parse("").expect("nothing is named");
    let unnamed_addr = serve_over_tcp(Observer::new(unnamed, Arc::clone(&unnamed_seen))).await;
    echo(&tcp_client(unnamed_addr, Some(FORWARDED))).await;

    let leg = observed(&unnamed_seen);
    assert_eq!(
        leg.client,
        Some(ClientAddress::Peer(LOOPBACK)),
        "an unnamed peer's forwarded address is ignored"
    );
    assert_eq!(
        leg.scheme,
        ClientScheme::Plaintext,
        "an unnamed peer cannot claim an encryption"
    );
}
