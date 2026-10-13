//! PROTOTYPE (throwaway). Not product code.
//!
//! ROUND ONE FILE, RETAINED BUT NO LONGER COMPILED. Round two rebuilt the
//! registry as a first-class crate (`proto-registry`, behind its `registry`
//! feature); this file is kept on disk as the record of round one's routing
//! table. It is not a module of `proto-host` any more.
//!
//! The registry: a routing table keyed by capability name, built at load time
//! from each plugin's published capability list, with schema validation in both
//! directions and per capability version gating.
//!
//! Assumptions made here, each also listed in `FINDINGS.md`:
//!
//! - capability names are globally unique and a duplicate fails the whole load,
//!   leaving the registry unchanged (two phase: stage, then commit);
//! - the schemas are JSON Schema 2020-12 documents that travel with the
//!   descriptor;
//! - the registry validates the request before the plugin sees it and the
//!   response before the caller sees it;
//! - a response that violates its own published schema is an internal error
//!   attributed to the plugin.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use crate::seam::{
    BoxFuture, Call, CapabilityKind, Payload, Plugin, PluginError,
};

/// Which capability versions this registry supports.
///
/// A per capability override plus a default. Strictly "the version the host was
/// built against" would be `default_max_version: 0`, but the prototype wants
/// most capabilities to register.
pub struct VersionPolicy {
    pub default_max_version: u32,
    pub max_versions: HashMap<String, u32>,
}

impl VersionPolicy {
    pub fn new(default_max_version: u32) -> Self {
        VersionPolicy {
            default_max_version,
            max_versions: HashMap::new(),
        }
    }

    pub fn max_version(&self, capability: &str) -> u32 {
        self.max_versions
            .get(capability)
            .copied()
            .unwrap_or(self.default_max_version)
    }
}

/// A capability the plugin published and the registry refuses to route.
#[derive(Debug, Clone)]
pub struct RefusedCapability {
    pub plugin: String,
    pub capability: String,
    pub advertised_version: u32,
    pub supported_version: u32,
}

/// What one successful load did.
#[derive(Debug)]
pub struct Registration {
    pub plugin: String,
    pub registered: Vec<String>,
    pub refused: Vec<RefusedCapability>,
}

/// Why a load failed. A failed load registers nothing.
#[derive(Debug)]
pub enum RegistrationError {
    DuplicateCapability {
        capability: String,
        existing_plugin: String,
        new_plugin: String,
    },
    BadSchema {
        capability: String,
        detail: String,
    },
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistrationError::DuplicateCapability {
                capability,
                existing_plugin,
                new_plugin,
            } => write!(
                f,
                "capability {capability} is already registered by {existing_plugin}; {new_plugin} cannot claim it"
            ),
            RegistrationError::BadSchema { capability, detail } => {
                write!(f, "capability {capability} published an unusable schema: {detail}")
            }
        }
    }
}

impl std::error::Error for RegistrationError {}

/// One routing table entry: the plugin, the kind and version it published, and
/// the two compiled validators.
pub struct Route {
    plugin: Arc<dyn Plugin>,
    plugin_name: String,
    kind: CapabilityKind,
    version: u32,
    request_schema: Value,
    response_schema: Value,
    request_validator: jsonschema::Validator,
    response_validator: jsonschema::Validator,
}

impl Route {
    pub fn plugin_name(&self) -> &str {
        &self.plugin_name
    }

    pub fn kind(&self) -> CapabilityKind {
        self.kind
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn request_schema(&self) -> &Value {
        &self.request_schema
    }

    pub fn response_schema(&self) -> &Value {
        &self.response_schema
    }

    /// Validates a request payload against the published request schema.
    ///
    /// Public because the benchmark calls it in a loop.
    pub fn validate_request(&self, payload: &Payload) -> Result<(), String> {
        let value: Value = serde_json::from_slice(payload.as_bytes())
            .map_err(|err| format!("the payload is not a JSON value: {err}"))?;
        first_error(&self.request_validator, &value)
    }

    /// Validates a response payload against the published response schema.
    pub fn validate_response(&self, payload: &Payload) -> Result<(), String> {
        let value: Value = serde_json::from_slice(payload.as_bytes())
            .map_err(|err| format!("the payload is not a JSON value: {err}"))?;
        first_error(&self.response_validator, &value)
    }
}

fn first_error(validator: &jsonschema::Validator, value: &Value) -> Result<(), String> {
    match validator.iter_errors(value).next() {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    }
}

/// The registry the daemon would hold.
pub struct PluginRegistry {
    routes: BTreeMap<String, Route>,
    plugins: Vec<Arc<dyn Plugin>>,
    refused: Vec<RefusedCapability>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        PluginRegistry::new()
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        PluginRegistry {
            routes: BTreeMap::new(),
            plugins: Vec::new(),
            refused: Vec::new(),
        }
    }

    /// Loads one plugin: reads its manifest and capability list, gates versions,
    /// compiles both schemas per capability, and stages every route.
    ///
    /// Nothing is written to the registry until every check has passed, so a
    /// load that fails on a duplicate leaves the routing table exactly as it
    /// was.
    pub fn register(
        &mut self,
        plugin: Arc<dyn Plugin>,
        policy: &VersionPolicy,
    ) -> Result<Registration, RegistrationError> {
        let plugin_name = plugin.manifest().name;
        let mut staged: Vec<(String, Route)> = Vec::new();
        let mut refused: Vec<RefusedCapability> = Vec::new();

        for descriptor in plugin.capabilities() {
            if let Some(existing) = self.routes.get(&descriptor.name) {
                return Err(RegistrationError::DuplicateCapability {
                    capability: descriptor.name,
                    existing_plugin: existing.plugin_name.clone(),
                    new_plugin: plugin_name,
                });
            }
            if staged.iter().any(|(name, _)| name == &descriptor.name) {
                return Err(RegistrationError::DuplicateCapability {
                    capability: descriptor.name,
                    existing_plugin: plugin_name.clone(),
                    new_plugin: plugin_name,
                });
            }

            let supported_version = policy.max_version(&descriptor.name);
            if descriptor.version > supported_version {
                refused.push(RefusedCapability {
                    plugin: plugin_name.clone(),
                    capability: descriptor.name,
                    advertised_version: descriptor.version,
                    supported_version,
                });
                continue;
            }

            let request_validator =
                compile(&descriptor.name, "request", &descriptor.request_schema)?;
            let response_validator =
                compile(&descriptor.name, "response", &descriptor.response_schema)?;
            staged.push((
                descriptor.name.clone(),
                Route {
                    plugin: Arc::clone(&plugin),
                    plugin_name: plugin_name.clone(),
                    kind: descriptor.kind,
                    version: descriptor.version,
                    request_schema: descriptor.request_schema,
                    response_schema: descriptor.response_schema,
                    request_validator,
                    response_validator,
                },
            ));
        }

        let registered: Vec<String> = staged.iter().map(|(name, _)| name.clone()).collect();
        for (name, route) in staged {
            self.routes.insert(name, route);
        }
        self.refused.extend(refused.iter().cloned());
        self.plugins.push(plugin);

        Ok(Registration {
            plugin: plugin_name,
            registered,
            refused,
        })
    }

    /// Routes one call.
    ///
    /// The request is validated synchronously, before the plugin is called at
    /// all. The response is validated inside the returned future, before the
    /// caller can see it.
    pub fn invoke<'a>(
        &'a self,
        capability: &'a str,
        payload: Payload,
        call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>> {
        let Some(route) = self.routes.get(capability) else {
            return Box::pin(async move {
                Err(PluginError::NotFound(format!(
                    "no capability named {capability} is registered"
                )))
            });
        };

        if let Err(detail) = route.validate_request(&payload) {
            let capability = capability.to_owned();
            return Box::pin(async move {
                Err(PluginError::InvalidArgument(format!(
                    "capability {capability}: the request violates its published schema: {detail}"
                )))
            });
        }

        Box::pin(async move {
            let response = route.plugin.invoke(capability, payload, call).await?;
            route.validate_response(&response).map_err(|detail| {
                PluginError::Internal(format!(
                    "plugin {} returned a response for capability {capability} that violates its \
                     published schema: {detail}",
                    route.plugin_name
                ))
            })?;
            Ok(response)
        })
    }

    /// The route for one capability, if it is registered.
    pub fn route(&self, capability: &str) -> Option<&Route> {
        self.routes.get(capability)
    }

    /// Every routed capability name, in order.
    pub fn capability_names(&self) -> Vec<String> {
        self.routes.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Every capability some plugin published and version gating refused.
    pub fn refused(&self) -> &[RefusedCapability] {
        &self.refused
    }

    /// Shuts every registered plugin down.
    pub fn shutdown(&self) -> Vec<(String, Result<(), PluginError>)> {
        self.plugins
            .iter()
            .map(|plugin| {
                let name = plugin.manifest().name;
                (name, plugin.shutdown())
            })
            .collect()
    }
}

fn compile(capability: &str, which: &str, schema: &Value) -> Result<jsonschema::Validator, RegistrationError> {
    jsonschema::validator_for(schema).map_err(|err| RegistrationError::BadSchema {
        capability: capability.to_owned(),
        detail: format!("{which} schema: {err}"),
    })
}
