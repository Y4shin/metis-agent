//! One sink for progress, events, logs and service requests.
//!
//! The host delivers every callback to the sink a caller installed when it
//! built the host, and it names the instance on every one of them. A host with
//! no sink of its own stays legal and records into the accessors the rest of
//! the suite reads, and a sink's panic is caught in the callback's own frame
//! and recorded rather than allowed to reach the plugin's poll.

#[path = "support/mod.rs"]
mod support;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use plugin_contract::abi::MetisStatus;
use plugin_fixture::payload::{Progress, TimerAnswer};
use plugin_runtime::sink::{LogLevel, Sink};
use plugin_runtime::{CallError, Host, Instance, PluginCall};

use support::{block_on, fixture, initialisation_body, request};

/// Which callback a test sink was told to panic on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Progress,
    Event,
    Log,
    Service,
}

/// One callback as the test sink received it.
///
/// The variants are the fields each callback kind carries: every one names the
/// instance, progress and a service request name their request, an event
/// carries its topic and payload and no request id, and a log carries its typed
/// level.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Delivered {
    Progress {
        instance: String,
        request_id: u64,
        payload: Vec<u8>,
    },
    Event {
        instance: String,
        topic: String,
        payload: Vec<u8>,
    },
    Log {
        instance: String,
        level: LogLevel,
        message: String,
    },
    Service {
        instance: String,
        service: String,
        request_id: u64,
        payload: Vec<u8>,
    },
}

/// The test sink: it records what it is handed, answers service requests with
/// the status it was built with, and panics on the kind it was told to.
#[derive(Clone)]
struct Recording {
    delivered: Arc<Mutex<Vec<Delivered>>>,
    answer: MetisStatus,
    panics_on: Option<Kind>,
}

impl Recording {
    fn new() -> Recording {
        Recording {
            delivered: Arc::new(Mutex::new(Vec::new())),
            answer: MetisStatus::Ok,
            panics_on: None,
        }
    }

    /// Everything delivered so far, in the order it arrived.
    fn delivered(&self) -> Vec<Delivered> {
        self.delivered.lock().unwrap().clone()
    }

    /// Every delivery of one kind, in order.
    fn of_kind(&self, want: &str) -> Vec<Delivered> {
        self.delivered()
            .into_iter()
            .filter(|delivered| match delivered {
                Delivered::Progress { .. } => want == "progress",
                Delivered::Event { .. } => want == "event",
                Delivered::Log { .. } => want == "log",
                Delivered::Service { .. } => want == "service",
            })
            .collect()
    }

    fn record(&self, kind: Kind, delivered: Delivered) {
        if self.panics_on == Some(kind) {
            panic!("test sink: a deliberate panic inside a {kind:?} callback");
        }
        self.delivered.lock().unwrap().push(delivered);
    }
}

impl Sink for Recording {
    fn progress(&self, instance: &str, request_id: u64, payload: Vec<u8>) {
        self.record(
            Kind::Progress,
            Delivered::Progress {
                instance: instance.to_string(),
                request_id,
                payload,
            },
        );
    }

    fn event(&self, instance: &str, topic: &str, payload: Vec<u8>) {
        self.record(
            Kind::Event,
            Delivered::Event {
                instance: instance.to_string(),
                topic: topic.to_string(),
                payload,
            },
        );
    }

    fn log(&self, instance: &str, level: LogLevel, message: &str) {
        self.record(
            Kind::Log,
            Delivered::Log {
                instance: instance.to_string(),
                level,
                message: message.to_string(),
            },
        );
    }

    fn service_request(
        &self,
        instance: &str,
        service: &str,
        request_id: u64,
        payload: Vec<u8>,
    ) -> MetisStatus {
        self.record(
            Kind::Service,
            Delivered::Service {
                instance: instance.to_string(),
                service: service.to_string(),
                request_id,
                payload,
            },
        );
        self.answer
    }
}

/// A host that names one instance, that instance, and the sink it delivers to.
///
/// The host handed in is the one the instance carries, which is the one that
/// names it: the sink is installed once, and the name is what makes every
/// callback say which plugin spoke.
fn named_instance(sink: &Recording, name: &str) -> (Host, Arc<Instance>) {
    let host = Host::with_sink(sink.clone()).for_instance(name);
    let loaded = fixture();
    let instance = block_on(
        loaded
            .init_plugin(&host, &initialisation_body(name))
            .expect("the fixture's initialisation entry starts"),
    )
    .expect("the fixture initialises with the configuration it declares");
    (host, instance)
}

/// Polls one call once.
///
/// The waker is the no-op one: every poll here is either ready or parked on a
/// service the test answers itself, so a wake has nothing to schedule. A real
/// daemon passes its own task waker, which is the same interface.
fn poll_once(call: &mut PluginCall) -> Poll<Result<Vec<u8>, CallError>> {
    let mut context = Context::from_waker(Waker::noop());
    Pin::new(call).poll(&mut context)
}

/// Invokes one capability and finishes it, answering any service it asks for.
///
/// A fixture capability that asks for a service parks until the host answers,
/// so the loop polls, answers what arrived with the sink's own answer, and
/// polls again.
fn finish(
    instance: &Arc<Instance>,
    sink: &Recording,
    capability: &str,
) -> Result<Vec<u8>, CallError> {
    let mut call = instance
        .invoke(capability, 1, &request("wait"))
        .expect("the future starts");
    let mut answered = false;
    loop {
        match poll_once(&mut call) {
            Poll::Ready(result) => return result,
            Poll::Pending => {
                let requests: Vec<(String, u64)> = sink
                    .delivered()
                    .into_iter()
                    .filter_map(|delivered| match delivered {
                        Delivered::Service {
                            service,
                            request_id,
                            ..
                        } => Some((service, request_id)),
                        _ => None,
                    })
                    .collect();
                for (service, request_id) in requests {
                    if service != "timer" || answered {
                        continue;
                    }
                    let answer = serde_json::to_vec(&TimerAnswer { delay_ms: 5 })
                        .expect("the fixture's timer answer serialises");
                    let _ = instance.complete_service(request_id, MetisStatus::Ok, &answer);
                    answered = true;
                }
            }
        }
    }
}

#[test]
fn each_callback_kind_is_delivered_with_the_instance_name_and_its_fields() {
    let sink = Recording::new();
    let (_host, instance) = named_instance(&sink, "fixture");

    // The fixture logs and emits an event while it initialises, so the two
    // lifecycle kinds arrive before any call is made.
    let echo = finish(&instance, &sink, "tool.echo").expect("the echo call succeeds");
    let echoed = support::decode(&echo);
    assert_eq!(echoed.echo, "wait");

    // `tool.pending` reports progress and asks for a timer, which `finish`
    // answers through the completion entry.
    finish(&instance, &sink, "tool.pending").expect("the pending call succeeds");

    let delivered = sink.delivered();
    assert!(
        !delivered.is_empty(),
        "the sink received nothing at all: {delivered:?}"
    );

    for entry in &delivered {
        let instance = match entry {
            Delivered::Progress { instance, .. }
            | Delivered::Event { instance, .. }
            | Delivered::Log { instance, .. }
            | Delivered::Service { instance, .. } => instance,
        };
        assert_eq!(
            instance, "fixture",
            "every callback names the instance it came from: {entry:?}"
        );
    }

    let events = sink.of_kind("event");
    let topics: Vec<&str> = events
        .iter()
        .map(|event| match event {
            Delivered::Event { topic, .. } => topic.as_str(),
            other => panic!("only an event is an event: {other:?}"),
        })
        .collect();
    assert_eq!(
        topics,
        vec!["fixture.ready", "fixture.answered"],
        "the event carries its topic, and an event has no request id to carry"
    );
    let ready = match &events[0] {
        Delivered::Event { topic, payload, .. } => (topic.clone(), payload.clone()),
        other => panic!("only an event is an event: {other:?}"),
    };
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&ready.1).expect("the event payload is JSON"),
        serde_json::json!({ "ready": true }),
        "the event carries the payload the plugin emitted"
    );

    let progress = sink.of_kind("progress");
    assert_eq!(progress.len(), 1, "one progress report: {progress:?}");
    match &progress[0] {
        Delivered::Progress {
            request_id,
            payload,
            ..
        } => {
            assert_eq!(*request_id, 1, "progress names the request it belongs to");
            assert_eq!(
                serde_json::from_slice::<Progress>(payload).expect("the progress payload is JSON"),
                Progress {
                    stage: "working".to_string()
                },
                "the progress carries the payload the plugin reported"
            );
        }
        other => panic!("only a progress report is a progress report: {other:?}"),
    }

    let services = sink.of_kind("service");
    assert_eq!(services.len(), 1, "one service request: {services:?}");
    match &services[0] {
        Delivered::Service {
            service,
            request_id,
            payload,
            ..
        } => {
            assert_eq!(service, "timer");
            assert_eq!(*request_id, 1, "the request names the id the answer needs");
            assert_eq!(
                serde_json::from_slice::<TimerAnswer>(payload).expect("the ask is JSON"),
                TimerAnswer { delay_ms: 5 },
                "the service request carries what the plugin asked for"
            );
        }
        other => panic!("only a service request is a service request: {other:?}"),
    }

    let logs = sink.of_kind("log");
    assert!(
        logs.iter().any(|log| matches!(
            log,
            Delivered::Log { level: LogLevel::Info, message, .. }
                if message == "fixture: initialised for fixture"
        )),
        "the fixture's own initialisation log arrived with its typed level: {logs:?}"
    );
    assert!(
        logs.iter().all(|log| matches!(log, Delivered::Log { .. })),
        "a log line is a log line"
    );
}
