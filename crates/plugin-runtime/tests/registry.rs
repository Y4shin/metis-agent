//! The runtime's registry seams, driven against the real fixture cdylib.

#[path = "support/mod.rs"]
mod support;

use std::sync::Arc;

use plugin_contract::abi::MetisStatus;
use plugin_fixture::{CAPABILITIES, NEWER_CAPABILITY, NEWER_CAPABILITY_VERSION};
use plugin_runtime::{CallError, Registry, RouteError};
use serde_json::json;

use support::{block_on, new_instance, request};

#[test]
fn the_loader_reads_the_declaration_and_the_version() {
    let (_, instance) = new_instance();
    assert_eq!(instance.loaded().abi(), (1, 2));
    assert!(instance.loaded().has_service_complete());
    let declared: Vec<&str> = instance
        .capabilities()
        .iter()
        .map(|capability| capability.name.as_str())
        .collect();
    assert_eq!(
        declared,
        CAPABILITIES.to_vec(),
        "the shared declaration parses what the plugin publishes"
    );
    // The fixture declares one capability at a version above the one a host of
    // this revision serves, which is what the per-capability gate is driven
    // with; every other descriptor declares version 1.
    assert!(
        instance.capabilities().iter().all(|capability| {
            let expected = if capability.name == NEWER_CAPABILITY {
                NEWER_CAPABILITY_VERSION
            } else {
                1
            };
            capability.kind == "tool"
                && capability.version == expected
                && capability.request_schema.is_object()
        }),
        "every descriptor carries its schemas inline"
    );
}

#[test]
fn routing_validates_both_directions() {
    let (host, instance) = new_instance();
    let mut registry = Registry::new();
    registry
        .register("fixture", Arc::clone(&instance))
        .expect("one instance registers");

    // A valid call: routed by name, validated, invoked, awaited, validated.
    let response = block_on(registry.call("fixture.tool.echo", &request("hello"), 1))
        .expect("the call succeeds");
    assert_eq!(support::decode(&response).echo, "hello");

    // The request direction: a body that violates the published schema is the
    // caller's mistake.
    let bad = serde_json::to_vec(&json!({ "echo": 5 })).unwrap();
    match block_on(registry.call("fixture.tool.echo", &bad, 2)) {
        Err(RouteError::InvalidArgument(detail)) => {
            assert!(detail.contains("schema"), "{detail}");
        }
        other => panic!("expected an invalid argument, got {other:?}"),
    }

    // The response direction: a body that violates the schema the plugin
    // published is the plugin's bug, attributed to it and never a quarantine.
    let capability = instance
        .capabilities()
        .iter()
        .find(|capability| capability.name == "tool.echo")
        .expect("tool.echo is declared");
    let wrong = serde_json::to_vec(&json!({ "echo": "fine", "bytes": "not a number" })).unwrap();
    let error = plugin_runtime::registry::validate_response(&capability.response_schema, &wrong)
        .expect_err("a response that breaks its own schema is refused");
    assert!(matches!(error, RouteError::InvalidResponse(_)));
    assert!(
        !instance.is_quarantined(),
        "a bad payload never quarantines"
    );

    assert!(
        host.events()
            .iter()
            .any(|topic| topic == "fixture.answered")
    );
    assert!(
        host.logs()
            .iter()
            .any(|line| line.contains("instance initialised"))
    );
}

#[test]
fn the_registry_refuses_duplicates_and_toggles_entries() {
    let (_, instance) = new_instance();
    let mut registry = Registry::new();
    registry.register("fixture", Arc::clone(&instance)).unwrap();

    let duplicate = registry.register("fixture", Arc::clone(&instance));
    assert!(
        matches!(
            duplicate,
            Err(plugin_runtime::RegistryError::DuplicateCapability { .. })
        ),
        "the same instance name may not be registered twice: {duplicate:?}"
    );

    // Two instances under two names keep two namespaces, which is what the
    // instance-qualified name is for.
    let (_, second) = new_instance();
    registry
        .register("second", second)
        .expect("a second instance under a second name");

    assert!(registry.enable("fixture", false));
    match registry
        .route("fixture.tool.echo")
        .map(|(entry, capability)| (entry.instance_name.clone(), capability.name.clone()))
    {
        Err(RouteError::Disabled(name)) => assert_eq!(name, "fixture.tool.echo"),
        other => panic!("a disabled entry is reported as disabled, got {other:?}"),
    }
    assert!(registry.enable("fixture", true));
    assert!(registry.route("fixture.tool.echo").is_ok());

    assert!(registry.unregister("fixture").is_some());
    match registry
        .route("fixture.tool.echo")
        .map(|(entry, capability)| (entry.instance_name.clone(), capability.name.clone()))
    {
        Err(RouteError::Unknown(name)) => assert_eq!(name, "fixture.tool.echo"),
        other => panic!("an unregistered entry is unknown, got {other:?}"),
    }
    assert!(registry.unregister("fixture").is_none());
}

#[test]
fn an_instance_name_carrying_a_dot_routes_like_any_other() {
    let (_, instance) = new_instance();
    let mut registry = Registry::new();
    registry
        .register("metis.echo", Arc::clone(&instance))
        .expect("a dotted instance name registers");

    // The routing map is keyed on the exact qualified name, so the owner comes
    // back unambiguously and the tool half is what follows it. Reading the tool
    // half from the first dot instead would look for "echo.tool.echo".
    let (entry, capability) = registry
        .route("metis.echo.tool.echo")
        .expect("a dotted instance name routes");
    assert_eq!(entry.instance_name, "metis.echo");
    assert_eq!(capability.name, "tool.echo");
    assert!(registry.gate("metis.echo.tool.echo", 1).is_ok());
    assert!(
        registry
            .validate_request("metis.echo.tool.echo", &request("hello"))
            .is_ok()
    );

    let response = block_on(registry.call("metis.echo.tool.echo", &request("hello"), 1))
        .expect("the call succeeds");
    assert_eq!(support::decode(&response).echo, "hello");

    // A neighbouring undotted name keeps its own namespace.
    let (_, second) = new_instance();
    registry
        .register("metis", second)
        .expect("the neighbour registers");
    let (owner, _) = registry
        .route("metis.tool.echo")
        .expect("the neighbour routes");
    assert_eq!(owner.instance_name, "metis");
    let (owner, _) = registry
        .route("metis.echo.tool.echo")
        .expect("the dotted name still routes");
    assert_eq!(owner.instance_name, "metis.echo");
}

#[test]
fn the_version_gate_is_per_capability() {
    let (_, instance) = new_instance();
    let mut registry = Registry::new();
    registry.register("fixture", instance).unwrap();

    registry
        .gate("fixture.tool.echo", 1)
        .expect("version 1 satisfies a requirement of 1");
    match registry.gate("fixture.tool.echo", 2) {
        Err(RouteError::Version {
            required, declared, ..
        }) => {
            assert_eq!((required, declared), (2, 1));
        }
        other => panic!("a too-old capability is refused on its own, got {other:?}"),
    }
    assert_eq!(
        registry.gate("fixture.tool.nope", 1).unwrap_err().status(),
        MetisStatus::NotFound
    );
}

#[test]
fn quarantine_stops_new_futures_and_unload_waits_for_the_ordering() {
    let (host, instance) = new_instance();
    let mut registry = Registry::new();
    registry.register("fixture", Arc::clone(&instance)).unwrap();

    // One call in flight: nothing may quiesce or unload.
    let call = instance
        .invoke("tool.pending", 7, &request("wait"))
        .expect("the future starts");
    assert_eq!(
        instance.shutdown().unwrap_err(),
        CallError::Payload(
            "futures are still in flight, so the instance cannot quiesce".to_string()
        )
    );
    let refused = Arc::clone(&instance).unload();
    assert!(
        refused.is_err(),
        "nothing is unloaded while a future is alive"
    );

    // The abandoned poll quarantines the instance: no new future is accepted,
    // and the library stays resident.
    instance.quarantine("a poll was abandoned by the watchdog");
    assert!(instance.is_quarantined());
    let refused_call = instance.invoke("tool.echo", 8, &request("after")).err();
    assert_eq!(refused_call, Some(CallError::Quarantined));

    // The late poll return is the reclaim: the caller drops the future, and
    // only then may the instance quiesce and unload.
    drop(call);
    assert!(instance.in_flight().is_empty());
    let entry = registry.unregister("fixture").expect("the entry was there");
    // The caller's own reference goes first, so the registry entry holds the
    // last one: unload is legal exactly when it does.
    drop(instance);
    let instance = entry.instance;
    instance.shutdown().expect("quiescence is legal now");
    assert!(registry.instance_names().is_empty());
    instance
        .unload()
        .expect("the instance unloads after quiescence and after every future is gone");

    let order = host.ordering();
    let quiesced = order
        .iter()
        .position(|event| event.starts_with("instance quiesced"));
    let destroyed = order
        .iter()
        .position(|event| event.starts_with("instance destroyed"));
    assert!(
        matches!((quiesced, destroyed), (Some(q), Some(d)) if q < d),
        "quiescence is recorded before the destroy: {order:?}"
    );
    assert!(
        order
            .iter()
            .any(|event| event.starts_with("unload refused")),
        "the refusal is recorded rather than inferred: {order:?}"
    );
}
