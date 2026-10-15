//! The sample plugin's end-to-end gate: a signed bundle in, a dispatched
//! capability, an event through the sink, and a clean release, all through the
//! entries the daemon uses.
//!
//! The bundle is the C fixture's own library beside a hand-written manifest, so
//! the format the gate packages is the format an author writes without a
//! generator. It is signed with the shared test signer's key, which exists only
//! in this suite, and every case here installs that key's anchor as a root
//! explicitly: the daemon's compiled-in root set is empty, which is what keeps
//! a shipped build from trusting a test key.
//!
//! Four properties shape the cases below.
//!
//! **The gate drives the public entries, not the raw table.** The load is
//! `load_bundle`, the dispatch goes through the registry the load built, and the
//! release is `Serving::release`, so a regression in the composed path fails
//! here even when every unit seam below it still passes.
//!
//! **The sink is the destination.** A host built with `Host::with_sink` records
//! nothing into its accessors, so the event the gate observes is read from the
//! sink the test installed, which is the same destination a daemon installs.
//!
//! **A refusing bundle is refused before anything runs.** The altered byte and
//! the unconfigured key are decided by the verifier, which is the first step of
//! the load, so both refusals name the signature rather than something the
//! plugin did.
//!
//! **The release is asserted on the ledger.** The order the release performs is
//! the ordering ledger's record rather than a report field, which is what makes
//! "the instance left the registry, then quiesced, then was destroyed, then the
//! library was unloaded" a check rather than a claim.

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use plugin_contract::abi::MetisStatus;
use plugin_runtime::Host;
use plugin_runtime::bundle::TrustStore;
use plugin_runtime::load::LoadError;
use plugin_runtime::sink::{LogLevel, Sink};
use serde_json::{Value, json};
use support::block_on;
use support::sample;

/// One callback as the gate's sink received it.
///
/// The gate reads the event the sample plugin emits and, on release, the log
/// line the C fixture writes when its instance is stopped. Progress and a
/// service request are recorded too, so a gate that started driving a capability
/// asking for a service would show it rather than lose it.
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

/// The gate's sink: it records everything it is handed and accepts a service
/// request so a capability that asks for one is never left parked.
#[derive(Clone, Default)]
struct Recording {
    delivered: Arc<Mutex<Vec<Delivered>>>,
}

impl Recording {
    fn new() -> Recording {
        Recording::default()
    }

    /// Everything delivered so far, in the order it arrived.
    fn delivered(&self) -> Vec<Delivered> {
        self.delivered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The events the sink was handed, in order.
    fn events(&self) -> Vec<(String, String, Vec<u8>)> {
        self.delivered()
            .into_iter()
            .filter_map(|delivered| match delivered {
                Delivered::Event {
                    instance,
                    topic,
                    payload,
                } => Some((instance, topic, payload)),
                _ => None,
            })
            .collect()
    }

    /// The log lines the sink was handed, in order.
    fn logs(&self) -> Vec<String> {
        self.delivered()
            .into_iter()
            .filter_map(|delivered| match delivered {
                Delivered::Log { message, .. } => Some(message),
                _ => None,
            })
            .collect()
    }

    fn record(&self, delivered: Delivered) {
        self.delivered
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(delivered);
    }
}

impl Sink for Recording {
    fn progress(&self, instance: &str, request_id: u64, payload: Vec<u8>) {
        self.record(Delivered::Progress {
            instance: instance.to_string(),
            request_id,
            payload,
        });
    }

    fn event(&self, instance: &str, topic: &str, payload: Vec<u8>) {
        self.record(Delivered::Event {
            instance: instance.to_string(),
            topic: topic.to_string(),
            payload,
        });
    }

    fn log(&self, instance: &str, level: LogLevel, message: &str) {
        self.record(Delivered::Log {
            instance: instance.to_string(),
            level,
            message: message.to_string(),
        });
    }

    fn service_request(
        &self,
        instance: &str,
        service: &str,
        request_id: u64,
        payload: Vec<u8>,
    ) -> MetisStatus {
        self.record(Delivered::Service {
            instance: instance.to_string(),
            service: service.to_string(),
            request_id,
            payload,
        });
        MetisStatus::Ok
    }
}

/// The deadline the gate's release is given, which is the caller's own
/// monotonic reading, as `Instance::cancel_all` already takes one.
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(1)
}

/// The request body the gate dispatches: the sample manifest declares an object
/// request schema, so the body is one.
fn request_body() -> Vec<u8> {
    serde_json::to_vec(&json!({ "echo": "gate" })).expect("the request serialises")
}

/// Where a line sits in a ledger, or a panic naming what the ledger carries.
fn position(ledger: &[String], needle: &str) -> usize {
    ledger
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("the ledger carries {needle:?}: {ledger:#?}"))
}

#[test]
fn the_sample_bundle_loads_dispatches_and_emits_an_event_through_the_sink() {
    let sink = Recording::new();
    let host = Host::with_sink(sink.clone());
    let (file, trust) = sample::pack(&sample::manifest(&sample::CAPABILITIES));

    let serving = sample::load(&host, file.path(), &trust);
    assert_eq!(
        serving.name(),
        sample::NAME,
        "the instance is registered under the name the manifest declares"
    );
    let routed = format!("{}.{}", sample::NAME, sample::READY_CAPABILITY);
    assert!(
        serving.registry().route(&routed).is_ok(),
        "the loaded sample plugin routes the capability the manifest declares"
    );

    // The dispatch goes through the registry the load built, which validates the
    // request and the response against the schemas the manifest published.
    let response = block_on(serving.registry().call(&routed, &request_body(), 1))
        .expect("the loaded sample plugin serves its ready capability");
    assert_eq!(
        serde_json::from_slice::<Value>(&response).expect("the response is JSON"),
        serde_json::from_str::<Value>(sample::READY_RESPONSE).expect("the literal is JSON"),
        "the plugin's own response came back through the registry"
    );

    // The event the plugin emitted during that call reached the sink the gate
    // installed, named with the instance the load registered it under.
    let events = sink.events();
    assert_eq!(
        events.len(),
        1,
        "the ready call emitted exactly one event: {events:?}"
    );
    let (instance, topic, payload) = events.first().expect("one event arrived");
    assert_eq!(instance, sample::NAME, "the event names its instance");
    assert_eq!(topic, sample::READY_TOPIC, "the event's topic is its own");
    assert_eq!(
        serde_json::from_slice::<Value>(payload).expect("the event body is JSON"),
        serde_json::from_str::<Value>(sample::READY_EVENT_BODY).expect("the literal is JSON"),
        "the event's body is the JSON the plugin emitted"
    );

    // The release performs the contract's order, and the ledger is the record.
    let report = block_on(serving.release(deadline()));
    assert!(
        report.released(),
        "the gate's instance is released cleanly: {}",
        report
            .refusal()
            .map(ToString::to_string)
            .unwrap_or_default()
    );
    assert!(
        report.resident().is_none(),
        "a released instance keeps no library reference"
    );
    let ledger = report.ledger();
    let unregistered = position(ledger, "unregistered");
    let quiesced = position(ledger, "instance quiesced after every future was dropped");
    let destroyed = position(ledger, "instance destroyed");
    let unloaded = position(ledger, "library unloaded");
    assert!(
        unregistered < quiesced && quiesced < destroyed && destroyed < unloaded,
        "the instance leaves its registry, quiesces, is destroyed and then has its \
         library unloaded: {ledger:#?}"
    );

    // The C fixture's own account of the host's buffer ownership reached the
    // sink, which is the plugin's record of the session rather than the host's.
    assert!(
        sink.logs()
            .iter()
            .any(|line| line.contains("CPLUGIN_ACCOUNT")),
        "the plugin's own account line reached the sink: {:#?}",
        sink.logs()
    );
}

#[test]
fn a_manifest_that_disagrees_with_the_library_is_refused_loudly() {
    // The gate's packaging step and its load entry compose into one check: a
    // manifest that omits a capability the library declares is refused by the
    // load, naming the capability that differed, so no bundle the gate builds
    // can disagree with the library it carries.
    let host = Host::new();
    let omitted: Vec<&str> = sample::CAPABILITIES
        .iter()
        .copied()
        .filter(|name| *name != "c.cancel")
        .collect();
    let (file, trust) = sample::pack(&sample::manifest(&omitted));

    match block_on(plugin_runtime::load::load_bundle(
        &host,
        file.path(),
        &sample::config(),
        &sample::host_context(),
        &trust,
    )) {
        Err(LoadError::CapabilityMismatch { capability, detail }) => {
            assert_eq!(
                capability, "c.cancel",
                "the refusal names the capability that differed"
            );
            assert!(
                detail.contains("not by the manifest"),
                "the refusal says which side is missing it: {detail}"
            );
        }
        Err(other) => panic!("a manifest omitting a capability is that refusal: {other}"),
        Ok(_) => panic!("a manifest the library disagrees with must not load"),
    }

    // The other direction: a manifest that declares a capability the library
    // does not have is the same refusal, naming that capability.
    let mut extra: Vec<&str> = sample::CAPABILITIES.to_vec();
    extra.push("c.not_in_the_library");
    let (file, trust) = sample::pack(&sample::manifest(&extra));

    match block_on(plugin_runtime::load::load_bundle(
        &host,
        file.path(),
        &sample::config(),
        &sample::host_context(),
        &trust,
    )) {
        Err(LoadError::CapabilityMismatch { capability, detail }) => {
            assert_eq!(capability, "c.not_in_the_library");
            assert!(
                detail.contains("not by the library"),
                "the refusal says which side is missing it: {detail}"
            );
        }
        Err(other) => panic!("a manifest declaring an absent capability is that refusal: {other}"),
        Ok(_) => panic!("a manifest the library disagrees with must not load"),
    }
}

#[test]
fn a_bundle_with_one_altered_byte_is_refused_before_it_is_loaded() {
    let host = Host::with_sink(Recording::new());
    let bytes = sample::bytes(&sample::manifest(&sample::CAPABILITIES));

    // One byte of the archive, which is the payload the signature covers, is
    // altered: the container still parses and the chain is the same one, so the
    // only thing wrong with the file is the byte.
    let mut altered = bytes.clone();
    let last = altered.len() - 1;
    altered[last] ^= 0x01;
    assert_ne!(altered, bytes, "the copy is not the original");
    let file = support::bundle::BundleFile::write(&altered);

    match block_on(plugin_runtime::load::load_bundle(
        &host,
        file.path(),
        &sample::config(),
        &sample::host_context(),
        &sample::trust(),
    )) {
        Err(LoadError::Trust(error)) => {
            assert!(
                error.is_trust_failure(),
                "an altered payload is a trust failure: {error}"
            );
        }
        Err(other) => panic!("an altered byte is refused by the signature: {other}"),
        Ok(_) => panic!("a bundle with an altered byte must not load"),
    }
}

#[test]
fn a_bundle_signed_by_a_key_that_is_not_a_root_is_refused() {
    let host = Host::with_sink(Recording::new());
    let chain = sample::untrusted_chain();
    let bytes = sample::bytes_signed_by(&chain, &sample::manifest(&sample::CAPABILITIES));
    let file = support::bundle::BundleFile::write(&bytes);

    // The trust store is the one the configured test key builds: the other
    // chain's anchor is not in it, so the bundle is signed by a key that is not
    // a configured root.
    match block_on(plugin_runtime::load::load_bundle(
        &host,
        file.path(),
        &sample::config(),
        &sample::host_context(),
        &sample::trust(),
    )) {
        Err(LoadError::Trust(error)) => {
            assert!(
                error.is_trust_failure(),
                "a signature no configured root anchors is a trust failure: {error}"
            );
        }
        Err(other) => panic!("an unconfigured key is refused by the verifier: {other}"),
        Ok(_) => panic!("a bundle no configured root anchors must not load"),
    }
}

#[test]
fn the_test_key_is_not_a_compiled_in_root() {
    // The daemon's default root set is the compiled-in one, and it is empty: a
    // shipped build trusts no test key, so the sample bundle has to be refused
    // by it.
    let default = TrustStore::default();
    assert!(
        default.roots().is_empty(),
        "the compiled-in root set trusts nothing: {:?}",
        default.roots()
    );

    let bytes = sample::bytes(&sample::manifest(&sample::CAPABILITIES));
    let error = plugin_runtime::bundle::verify_bundle(&bytes, &default)
        .expect_err("the test key is not a compiled-in root");
    assert!(
        error.is_trust_failure(),
        "the refusal names the untrusted signature: {error}"
    );
}
