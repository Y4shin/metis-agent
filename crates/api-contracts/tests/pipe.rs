//! The Windows local transport driven over a real named pipe.
//!
//! The serve-and-connect test is `#[cfg(windows)]`: the `windows` CI job
//! compiles it for the Windows target and no runner in this effort executes it,
//! so what it would prove on a Windows host is recorded as a gap below and in
//! `docs/testing.md`. On every other host the explicit skip test at the bottom
//! runs instead, so a Linux `cargo test` reports the missing pipe coverage
//! rather than looking complete.
//!
//! The pipe itself, the name rule, and the descriptor live in
//! `api_contracts::transport`; this file only drives the served path.

/// The pipe, the generated client, and the service it echoes through.
#[cfg(windows)]
mod pipe {
    use std::sync::Arc;
    use std::time::Duration;

    use api_contracts::connect::metis::v1::{DaemonService, DaemonServiceClient, DaemonServiceExt};
    use api_contracts::proto::metis::v1::{
        CancelOutcome, CancelRequest, CancelResponse, EchoRequest, EchoResponse, Event,
        StreamEventsRequest,
    };
    use api_contracts::transport::{self, PIPE_PREFIX, PipeName};
    use connectrpc::client::ClientConfig;
    use connectrpc::{
        RequestContext, Response, Router, ServiceRequest, ServiceResult, ServiceStream,
    };
    use tokio::net::windows::named_pipe::ServerOptions;

    /// The service the pipe serves. Only the echo path is called; the other
    /// methods exist because the generated trait declares them.
    struct PipeDaemon;

    #[allow(refining_impl_trait)]
    impl DaemonService for PipeDaemon {
        async fn echo(
            &self,
            _ctx: RequestContext,
            request: ServiceRequest<'_, EchoRequest>,
        ) -> ServiceResult<EchoResponse> {
            Response::ok(EchoResponse {
                message: request.message.to_owned(),
                received_at: String::new(),
                ..Default::default()
            })
        }

        async fn pair(
            &self,
            _ctx: RequestContext,
            _request: ServiceRequest<'_, api_contracts::proto::metis::v1::PairRequest>,
        ) -> ServiceResult<api_contracts::proto::metis::v1::PairResponse> {
            Err(connectrpc::ConnectError::unimplemented("pair"))
        }

        async fn stream_events(
            &self,
            _ctx: RequestContext,
            _request: ServiceRequest<'_, StreamEventsRequest>,
        ) -> ServiceResult<ServiceStream<Event>> {
            Err(connectrpc::ConnectError::unimplemented("stream_events"))
        }

        async fn cancel(
            &self,
            _ctx: RequestContext,
            _request: ServiceRequest<'_, CancelRequest>,
        ) -> ServiceResult<CancelResponse> {
            Response::ok(CancelResponse {
                outcome: CancelOutcome::AlreadyFinished.into(),
                ..Default::default()
            })
        }
    }

    /// The router the pipe serves, built the way every other front-end is
    /// handed one.
    fn router() -> Router {
        Arc::new(PipeDaemon).register(Router::new())
    }

    /// A pipe name unique to this process, so two runs of this binary cannot
    /// share one pipe.
    fn test_pipe_name() -> PipeName {
        PipeName::parse(&format!("{PIPE_PREFIX}metis-test-{}", std::process::id()))
            .expect("the test builds a local pipe name")
    }

    /// The serve-and-connect seam: the service is served over a duplex
    /// byte-mode named pipe and one echo request round-trips through the
    /// connector, so the pipe is a third front-end over the same connection
    /// body rather than a path of its own.
    #[tokio::test]
    async fn the_echo_request_round_trips_over_the_pipe() {
        let name = test_pipe_name();
        let served = name.clone();
        tokio::spawn(async move {
            if let Err(err) = transport::serve_pipe(served, router()).await {
                eprintln!("pipe listener error: {err}");
            }
        });

        let response = echo_over_the_pipe(&name, "hello over the pipe").await;
        assert_eq!(response.message, "hello over the pipe");

        // The listener took `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a second
        // server asking to create the first instance of the same name is
        // refused while the daemon runs. That is what makes a squatter fail
        // loudly instead of quietly coexisting with the daemon.
        let squatter = ServerOptions::new()
            .first_pipe_instance(true)
            .create(name.as_str());
        assert!(
            squatter.is_err(),
            "the first instance of the pipe belongs to the listener"
        );
    }

    /// Call echo over a freshly built connection, retrying while the listener
    /// creates its first instance: the listener is spawned, so the pipe does
    /// not exist yet when the test starts. A failure is retried to the same
    /// deadline rather than swallowed, so a served refusal ends the test with
    /// that error named.
    async fn echo_over_the_pipe(name: &PipeName, message: &str) -> EchoResponse {
        let request = EchoRequest {
            message: message.to_owned(),
            ..Default::default()
        };
        let mut last = None;
        for _ in 0..100 {
            let transport = transport::pipe_transport(name).shared(64);
            let client = DaemonServiceClient::new(
                transport,
                ClientConfig::new(transport::local_authority()),
            );
            match client.echo(request.clone()).await {
                Ok(response) => return response.into_owned(),
                Err(err) => last = Some(err),
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the pipe never answered an echo request: {last:?}");
    }
}

/// The local half of the compile-and-skip rule: on a host with no named pipe
/// front-end this test runs and says so, so a Linux `cargo test` reports the
/// missing coverage instead of silently having no pipe test at all.
#[cfg(not(windows))]
#[test]
fn the_pipe_test_needs_a_windows_host() {
    eprintln!(
        "[skip] the Windows named-pipe serve-and-connect test lives at \
         `crates/api-contracts/tests/pipe.rs` and runs only on a Windows host; this host is \
         {}. Run `cargo test -p api-contracts --test pipe` there to execute it.",
        std::env::consts::OS
    );
    assert_ne!(std::env::consts::OS, "windows");
}
