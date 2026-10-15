//! The registry: what a daemon routes through, and what it validates with.
//!
//! A registry entry is one loaded instance and the capability declaration it
//! published. Names are instance-qualified as `<instance-name>.<tool-name>`, so
//! the same plugin loaded twice keeps two namespaces, and the plugin itself
//! only ever names the tool half.
//!
//! Two gates live here and they are different in kind. The *version* gate is
//! per capability and reads a descriptor, so one outdated capability does not
//! fail a whole plugin. The *presence* gate is the table's: a capability can
//! only be routed to an instance whose declaration carries it.
//!
//! Validation runs in both directions and the two directions differ in what a
//! failure means. A request that does not match the capability's published
//! schema is the caller's mistake and comes back as `invalid_argument`. A
//! response that does not match the plugin's own published schema is that
//! plugin's bug: it is an internal error attributed to the plugin, and it does
//! not quarantine it, because a wrong payload is not a stuck poll.

use std::collections::BTreeMap;
use std::sync::Arc;

use plugin_contract::abi::MetisStatus;
use plugin_contract::capability::{CapabilityDescriptor, CapabilityList};
use serde_json::Value;

use crate::Instance;

/// Parses the capability list payload body.
///
/// The declaration is the shared `plugin_contract::capability` type, so the
/// bytes a plugin publishes and the values this registry routes with are the
/// same shape by construction rather than by two types kept in step by a test.
///
/// A declaration that is not JSON, or that declares nothing, is refused here
/// rather than at the first call: an instance whose declaration cannot be
/// understood cannot be routed.
pub fn parse_capabilities(body: &[u8]) -> Result<Vec<CapabilityDescriptor>, crate::CallError> {
    let list: CapabilityList = serde_json::from_slice(body).map_err(|err| {
        crate::CallError::Payload(format!(
            "the capability list is not the declared JSON: {err}"
        ))
    })?;
    if list.capabilities.is_empty() {
        return Err(crate::CallError::Payload(
            "the plugin declares no capabilities".to_string(),
        ));
    }
    Ok(list.capabilities)
}

/// Whether a real JSON body satisfies a published schema.
pub fn validate_request(schema: &Value, body: &[u8]) -> Result<(), RouteError> {
    validate(schema, body, Direction::Request)
}

/// Whether a plugin's response satisfies the schema the plugin published for it.
pub fn validate_response(schema: &Value, body: &[u8]) -> Result<(), RouteError> {
    validate(schema, body, Direction::Response)
}

/// Refuses a capability whose kind this build does not know.
///
/// One capability at a time, for the same reason the version gate is per
/// capability: an unknown kind is that one capability's problem, not the
/// plugin's, so the rest of the declaration keeps working. The declaration's
/// `kind` is a string precisely so a newer plugin's kind parses at all and
/// lands here instead of failing the whole list.
fn kind_gate(qualified: &str, capability: &CapabilityDescriptor) -> Result<(), RouteError> {
    if capability.known_kind().is_none() {
        return Err(RouteError::UnknownKind {
            name: qualified.to_string(),
            kind: capability.kind.clone(),
        });
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Direction {
    Request,
    Response,
}

fn validate(schema: &Value, body: &[u8], direction: Direction) -> Result<(), RouteError> {
    // A body that is not JSON, or one that does not match, is the same failure
    // from the schema's point of view.
    let instance: Value = match serde_json::from_slice(body) {
        Ok(instance) => instance,
        Err(err) => {
            let detail = format!("the payload is not JSON: {err}");
            return Err(match direction {
                Direction::Request => RouteError::InvalidArgument(detail),
                Direction::Response => RouteError::InvalidResponse(detail),
            });
        }
    };
    if jsonschema::is_valid(schema, &instance) {
        return Ok(());
    }
    let detail = format!("the payload does not match the published schema: {instance}");
    Err(match direction {
        Direction::Request => RouteError::InvalidArgument(detail),
        Direction::Response => RouteError::InvalidResponse(detail),
    })
}

/// One registered instance.
pub struct Entry {
    /// The instance name the caller registered it under.
    pub instance_name: String,
    /// The instance itself.
    pub instance: Arc<Instance>,
    /// Whether the entry is enabled. A disabled entry still resolves, so a
    /// caller can tell "turned off" from "never loaded".
    pub enabled: bool,
}

/// Why a registry entry that the host switched off carries as its reason.
///
/// The registry's `enable` is a flag with no policy behind it: the daemon decides
/// why an entry is off, and this is what a caller reads when the daemon gave no
/// reason of its own. A plugin that start-up disabled carries its own reason
/// instead, which is why the refusal holds one.
const DISABLED_BY_HOST: &str = "the host turned this entry off";

/// What a route lookup can fail with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// Nothing is registered under that name.
    Unknown(String),
    /// The entry exists and is disabled.
    ///
    /// The name is what was asked for and the reason is why nothing served it,
    /// so a caller can tell "turned off" from "never loaded" and can report why.
    Disabled {
        /// The qualified capability name the caller asked for.
        name: String,
        /// Why the entry is disabled.
        reason: String,
    },
    /// The declaration does not carry a version the caller can accept.
    Version {
        name: String,
        required: u32,
        declared: u32,
    },
    /// The declaration names a kind this build does not know, so this one
    /// capability cannot be served while the rest of the plugin stays usable.
    UnknownKind {
        /// The qualified capability name.
        name: String,
        /// The kind the plugin declared.
        kind: String,
    },
    /// The request body did not match the capability's published schema.
    InvalidArgument(String),
    /// The response body did not match the plugin's published schema.
    InvalidResponse(String),
    /// The call itself failed.
    Call(crate::CallError),
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::Unknown(name) => write!(f, "no capability named {name}"),
            RouteError::Disabled { name, reason } => {
                write!(f, "the entry for {name} is disabled: {reason}")
            }
            RouteError::Version {
                name,
                required,
                declared,
            } => write!(
                f,
                "{name} declares version {declared} and version {required} was required"
            ),
            RouteError::UnknownKind { name, kind } => write!(
                f,
                "{name} declares kind {kind}, which this build does not know"
            ),
            RouteError::InvalidArgument(detail) => write!(f, "invalid argument: {detail}"),
            RouteError::InvalidResponse(detail) => {
                write!(
                    f,
                    "the plugin published a schema its own response violates: {detail}"
                )
            }
            RouteError::Call(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for RouteError {}

impl From<crate::CallError> for RouteError {
    fn from(error: crate::CallError) -> Self {
        RouteError::Call(error)
    }
}

impl RouteError {
    /// The status this failure carries across the boundary.
    pub fn status(&self) -> MetisStatus {
        match self {
            RouteError::Unknown(_) => MetisStatus::NotFound,
            RouteError::Disabled { .. } => MetisStatus::PermissionDenied,
            RouteError::Version { .. } => MetisStatus::Unsupported,
            RouteError::UnknownKind { .. } => MetisStatus::Unsupported,
            RouteError::InvalidArgument(_) => MetisStatus::InvalidArgument,
            RouteError::InvalidResponse(_) => MetisStatus::Internal,
            RouteError::Call(crate::CallError::Status(status)) => *status,
            RouteError::Call(_) => MetisStatus::Internal,
        }
    }
}

/// What registration can refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// The instance declares no capabilities.
    NoCapabilities(String),
    /// A capability name is already taken by another entry.
    DuplicateCapability {
        /// The qualified name that collided.
        name: String,
        /// The instance that already owns it.
        owner: String,
    },
    /// Two capabilities inside one declaration share a name.
    DuplicateInDeclaration(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::NoCapabilities(name) => {
                write!(f, "{name} declares no capabilities")
            }
            RegistryError::DuplicateCapability { name, owner } => {
                write!(f, "{name} is already registered by {owner}")
            }
            RegistryError::DuplicateInDeclaration(name) => {
                write!(f, "{name} declares the same capability name twice")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// The routing table.
#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<String, Entry>,
    /// Qualified capability name to the instance name that owns it, which is
    /// what makes duplicate refusal possible without scanning every entry.
    routes: BTreeMap<String, String>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Registers one instance under a name.
    ///
    /// A capability name is instance-qualified as `<instance>.<tool>`, and two
    /// entries may not claim the same qualified name: a duplicate is refused
    /// here rather than resolved by load order.
    pub fn register(
        &mut self,
        instance_name: &str,
        instance: Arc<Instance>,
    ) -> Result<(), RegistryError> {
        if instance.capabilities().is_empty() {
            return Err(RegistryError::NoCapabilities(instance_name.to_string()));
        }
        if self.entries.contains_key(instance_name) {
            return Err(RegistryError::DuplicateCapability {
                name: instance_name.to_string(),
                owner: instance_name.to_string(),
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for capability in instance.capabilities() {
            if !seen.insert(capability.name.clone()) {
                return Err(RegistryError::DuplicateInDeclaration(format!(
                    "{instance_name}.{}",
                    capability.name
                )));
            }
            let qualified = format!("{instance_name}.{}", capability.name);
            if let Some(owner) = self.routes.get(&qualified) {
                return Err(RegistryError::DuplicateCapability {
                    name: qualified,
                    owner: owner.clone(),
                });
            }
        }
        for capability in instance.capabilities() {
            self.routes.insert(
                format!("{instance_name}.{}", capability.name),
                instance_name.to_string(),
            );
        }
        self.entries.insert(
            instance_name.to_string(),
            Entry {
                instance_name: instance_name.to_string(),
                instance,
                enabled: true,
            },
        );
        Ok(())
    }

    /// Removes an entry and its routes, returning it so the caller can unload
    /// it after every future is gone.
    pub fn unregister(&mut self, instance_name: &str) -> Option<Entry> {
        let entry = self.entries.remove(instance_name)?;
        self.routes
            .retain(|_, owner| owner.as_str() != instance_name);
        Some(entry)
    }

    /// Turns an entry on or off, reporting whether it was there.
    pub fn enable(&mut self, instance_name: &str, enabled: bool) -> bool {
        match self.entries.get_mut(instance_name) {
            Some(entry) => {
                entry.enabled = enabled;
                true
            }
            None => false,
        }
    }

    /// Every instance name, in order.
    pub fn instance_names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Resolves a qualified capability name to its entry and declaration.
    ///
    /// A capability whose kind this build does not know is refused here, one
    /// capability at a time, for the same reason the version gate is per
    /// capability: an unknown kind is this capability's problem, not the
    /// plugin's, so the rest of the declaration keeps working.
    pub fn route(&self, qualified: &str) -> Result<(&Entry, &CapabilityDescriptor), RouteError> {
        let owner = self
            .routes
            .get(qualified)
            .ok_or_else(|| RouteError::Unknown(qualified.to_string()))?;
        let entry = self
            .entries
            .get(owner)
            .ok_or_else(|| RouteError::Unknown(qualified.to_string()))?;
        if !entry.enabled {
            return Err(RouteError::Disabled {
                name: qualified.to_string(),
                reason: DISABLED_BY_HOST.to_string(),
            });
        }
        // The owner is the routing map's own answer for this exact key, so the
        // tool half is what follows the owner and its separator. Splitting on
        // the first dot instead would read an instance name that carries a dot
        // as the instance plus part of its own name.
        let tool = qualified
            .strip_prefix(owner.as_str())
            .and_then(|rest| rest.strip_prefix('.'))
            .unwrap_or(qualified);
        let capability = entry
            .instance
            .capabilities()
            .iter()
            .find(|capability| capability.name == tool)
            .ok_or_else(|| RouteError::Unknown(qualified.to_string()))?;
        kind_gate(qualified, capability)?;
        Ok((entry, capability))
    }

    /// The per-capability version gate.
    ///
    /// It reads the descriptor the plugin published, not the table, so a
    /// capability that is too old is refused on its own while the rest of the
    /// plugin keeps working.
    pub fn gate(&self, qualified: &str, required: u32) -> Result<(), RouteError> {
        let (_, capability) = self.route(qualified)?;
        if capability.version < required {
            return Err(RouteError::Version {
                name: qualified.to_string(),
                required,
                declared: capability.version,
            });
        }
        Ok(())
    }

    /// Validates a request against a capability's published schema.
    pub fn validate_request(&self, qualified: &str, body: &[u8]) -> Result<(), RouteError> {
        let (_, capability) = self.route(qualified)?;
        validate_request(&capability.request_schema, body)
    }

    /// Validates a plugin's response against the schema the plugin published.
    pub fn validate_response(&self, qualified: &str, body: &[u8]) -> Result<(), RouteError> {
        let (_, capability) = self.route(qualified)?;
        validate_response(&capability.response_schema, body)
    }

    /// The launch-and-await API: route, validate the request, start the future,
    /// poll it, validate the response.
    ///
    /// The future this returns is the plugin's own, polled by whatever runtime
    /// the caller has. This crate starts none.
    pub async fn call(
        &self,
        qualified: &str,
        body: &[u8],
        request_id: u64,
    ) -> Result<Vec<u8>, RouteError> {
        let (entry, capability) = self.route(qualified)?;
        validate_request(&capability.request_schema, body)?;
        let call = entry.instance.invoke(&capability.name, request_id, body)?;
        let response = call.await?;
        validate_response(&capability.response_schema, &response)?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn descriptor(kind: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: "tool.probe".to_string(),
            kind: kind.to_string(),
            version: 1,
            request_schema: json!({"type": "object"}),
            response_schema: json!({"type": "object"}),
        }
    }

    #[test]
    fn a_kind_this_build_knows_passes_the_gate() {
        for kind in ["tool", "provider"] {
            assert_eq!(
                kind_gate("inst.tool.probe", &descriptor(kind)),
                Ok(()),
                "{kind} is a kind this build knows"
            );
        }
    }

    #[test]
    fn a_kind_this_build_does_not_know_is_refused_with_unsupported() {
        let error = kind_gate("inst.tool.probe", &descriptor("widget"))
            .expect_err("an unknown kind is refused");
        assert_eq!(
            error,
            RouteError::UnknownKind {
                name: "inst.tool.probe".to_string(),
                kind: "widget".to_string(),
            }
        );
        assert_eq!(error.status(), MetisStatus::Unsupported);
    }
}
