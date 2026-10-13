//! PROTOTYPE (throwaway). Not product code.
//!
//! The plugin registry proper, behind the `registry` feature.
//!
//! This is the first-class type `plugin-runtime` owns in the rough plan: it
//! loads or registers a plugin, validates its manifest, delivers configuration,
//! discovers capabilities (fallibly), resolves their schemas by id, gates
//! versions, exposes enable and disable state, lists plugins and instances,
//! unregisters a plugin, and routes calls by capability name across every
//! enabled plugin.
//!
//! Round two's five fixes live here or are exercised through here:
//!
//! 1. capability listing is fallible (`Plugin::capabilities` returns `Result`),
//!    and a failed listing fails the load with [`RegistryError::Discovery`];
//! 2. the failure paths are distinct [`PluginError`] variants, not one string;
//! 3. descriptors reference schemas by id and the registry fetches them in one
//!    batched call, so the listing stays small;
//! 4. [`RoutingMode`] chooses between global uniqueness and namespacing, so the
//!    trade-off is measurable rather than argued;
//! 5. the module is behind a feature, so the primitives build has no serde and
//!    no validator.
//!
//! Packaging concerns deliberately left out (they belong to the real
//! `plugin-runtime`, not to this prototype): bundle extraction, signature and
//! integrity checks, platform and architecture selection inside a bundle, and
//! hot unload of a library that still has live instances.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::{
    BoxFuture, Call, CapabilityDescriptor, CapabilityKind, Payload, Plugin, PluginError,
    PluginManifest, VersionPolicy,
};

/// Loads a plugin from a library path.
///
/// Injected, so the registry owns no `libloading` and the primitives build
/// stays free of it. `plugin-runtime` would supply the real one.
pub type PluginLoader = Arc<dyn Fn(&Path) -> Result<Arc<dyn Plugin>, String> + Send + Sync>;

/// How capability routing keys are formed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    /// The routing key is the capability name, and a duplicate fails the load.
    /// This is round one's chosen rule.
    GlobalUnique,
    /// The routing key is `plugin_id/capability`, so two plugins may publish
    /// the same capability name.
    Namespaced,
}

/// Why a load failed. A failed load registers nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// A plugin could not be loaded from a path.
    LoadFailed { plugin: String, detail: String },
    /// The manifest is missing a field or targets another ABI major.
    ManifestInvalid { plugin: String, detail: String },
    /// The plugin id is already registered.
    DuplicatePlugin { plugin: String },
    /// Capability discovery failed. Issue 1: this fails the load.
    Discovery { plugin: String, detail: String },
    /// Delivering configuration failed.
    Configure { plugin: String, detail: String },
    /// A schema id the plugin advertised could not be resolved.
    SchemaFetch { plugin: String, detail: String },
    /// A published schema did not compile.
    BadSchema {
        plugin: String,
        capability: String,
        detail: String,
    },
    /// A capability name is already routed, and the mode is `GlobalUnique`.
    DuplicateCapability {
        capability: String,
        existing_plugin: String,
        new_plugin: String,
    },
    /// No plugin with this id is registered.
    UnknownPlugin { plugin: String },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::LoadFailed { plugin, detail } => {
                write!(f, "could not load {plugin}: {detail}")
            }
            RegistryError::ManifestInvalid { plugin, detail } => {
                write!(f, "manifest for {plugin} is invalid: {detail}")
            }
            RegistryError::DuplicatePlugin { plugin } => {
                write!(f, "a plugin with id {plugin} is already registered")
            }
            RegistryError::Discovery { plugin, detail } => {
                write!(f, "capability discovery for {plugin} failed: {detail}")
            }
            RegistryError::Configure { plugin, detail } => {
                write!(f, "configuring {plugin} failed: {detail}")
            }
            RegistryError::SchemaFetch { plugin, detail } => {
                write!(f, "schema fetch for {plugin} failed: {detail}")
            }
            RegistryError::BadSchema {
                plugin,
                capability,
                detail,
            } => write!(f, "{plugin} published an unusable schema for {capability}: {detail}"),
            RegistryError::DuplicateCapability {
                capability,
                existing_plugin,
                new_plugin,
            } => write!(
                f,
                "capability {capability} is already routed by {existing_plugin}; {new_plugin} cannot claim it"
            ),
            RegistryError::UnknownPlugin { plugin } => {
                write!(f, "no plugin with id {plugin} is registered")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// A capability a plugin published and version gating refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedCapability {
    pub plugin: String,
    pub capability: String,
    pub advertised_version: u32,
    pub supported_version: u32,
}

/// One registered plugin, as [`PluginRegistry::list`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    /// The instance handle. One per plugin id in this prototype.
    pub instance: String,
    pub id: String,
    pub version: String,
    pub abi_major: u32,
    pub enabled: bool,
    /// The capability names (bare, not namespaced) this plugin published.
    pub capabilities: Vec<String>,
}

/// One routing table entry.
pub struct Route {
    plugin: Arc<dyn Plugin>,
    plugin_id: String,
    capability: String,
    kind: CapabilityKind,
    version: u32,
    request_schema: Value,
    response_schema: Value,
    request_validator: jsonschema::Validator,
    response_validator: jsonschema::Validator,
}

impl Route {
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn capability(&self) -> &str {
        &self.capability
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
    /// Public because the benchmark calls it in a loop.
    pub fn validate_request(&self, payload: &Payload) -> Result<(), String> {
        let value: Value = serde_json::from_slice(payload.as_bytes())
            .map_err(|err| format!("the payload is not a JSON value: {err}"))?;
        first_error(&self.request_validator, &value)
    }

    /// Validates a response payload against the published response schema.
    /// Public because the benchmark calls it in a loop.
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

struct Entry {
    instance: String,
    id: String,
    manifest: PluginManifest,
    plugin: Arc<dyn Plugin>,
    config: Value,
    capabilities: Vec<String>,
    enabled: bool,
}

/// The registry the daemon would hold.
pub struct PluginRegistry {
    routing: RoutingMode,
    routes: BTreeMap<String, Route>,
    refused: Vec<RefusedCapability>,
    refused_keys: BTreeMap<String, RefusedCapability>,
    plugins: BTreeMap<String, Entry>,
    loader: Option<PluginLoader>,
    next_instance: u64,
}

impl PluginRegistry {
    pub fn new(routing: RoutingMode) -> Self {
        PluginRegistry {
            routing,
            routes: BTreeMap::new(),
            refused: Vec::new(),
            refused_keys: BTreeMap::new(),
            plugins: BTreeMap::new(),
            loader: None,
            next_instance: 0,
        }
    }

    pub fn routing(&self) -> RoutingMode {
        self.routing
    }

    pub fn set_loader(&mut self, loader: PluginLoader) {
        self.loader = Some(loader);
    }

    /// The routing key for one capability under the chosen mode.
    pub fn key_for(&self, plugin_id: &str, capability: &str) -> String {
        match self.routing {
            RoutingMode::GlobalUnique => capability.to_owned(),
            RoutingMode::Namespaced => format!("{plugin_id}/{capability}"),
        }
    }

    /// Loads a plugin from a library path through the injected loader.
    pub fn load(
        &mut self,
        path: &Path,
        configuration: Value,
        policy: &VersionPolicy,
    ) -> Result<String, RegistryError> {
        let Some(loader) = self.loader.clone() else {
            return Err(RegistryError::LoadFailed {
                plugin: path.display().to_string(),
                detail: "no loader is installed".to_owned(),
            });
        };
        let plugin = loader(path).map_err(|detail| RegistryError::LoadFailed {
            plugin: path.display().to_string(),
            detail,
        })?;
        self.register(plugin, configuration, policy)
    }

    /// Registers a plugin, from a library path or an in-memory one.
    ///
    /// Two phase: every check runs against staged state, and nothing is
    /// committed until all of them pass, so a failed register leaves the
    /// routing table exactly as it was.
    pub fn register(
        &mut self,
        plugin: Arc<dyn Plugin>,
        configuration: Value,
        policy: &VersionPolicy,
    ) -> Result<String, RegistryError> {
        let manifest = plugin.manifest();
        if manifest.id.trim().is_empty() {
            return Err(RegistryError::ManifestInvalid {
                plugin: "<unnamed>".to_owned(),
                detail: "the plugin id is empty".to_owned(),
            });
        }
        if manifest.version.trim().is_empty() {
            return Err(RegistryError::ManifestInvalid {
                plugin: manifest.id.clone(),
                detail: "the plugin version is empty".to_owned(),
            });
        }
        if manifest.abi_major != 1 {
            return Err(RegistryError::ManifestInvalid {
                plugin: manifest.id.clone(),
                detail: format!(
                    "the plugin targets ABI major {}, this host speaks ABI major 1",
                    manifest.abi_major
                ),
            });
        }
        if self.plugins.contains_key(&manifest.id) {
            return Err(RegistryError::DuplicatePlugin {
                plugin: manifest.id.clone(),
            });
        }

        // Configuration travels before discovery, so a plugin can shape what it
        // publishes from what it was configured with.
        let config_bytes = serde_json::to_vec(&configuration).unwrap_or_else(|_| b"{}".to_vec());
        plugin
            .configure(&config_bytes)
            .map_err(|err| RegistryError::Configure {
                plugin: manifest.id.clone(),
                detail: err.to_string(),
            })?;

        // Issue 1: a failed listing is a typed load failure, not a plugin with
        // zero capabilities.
        let descriptors = plugin.capabilities().map_err(|err| RegistryError::Discovery {
            plugin: manifest.id.clone(),
            detail: err.to_string(),
        })?;

        // Stage every capability, gating versions and collecting schema ids.
        let mut staged: Vec<(String, CapabilityDescriptor)> = Vec::new();
        let mut refused: Vec<RefusedCapability> = Vec::new();
        let mut schema_ids: Vec<String> = Vec::new();
        for descriptor in descriptors {
            let key = self.key_for(&manifest.id, &descriptor.name);
            if let Some(existing) = self.routes.get(&key) {
                return Err(RegistryError::DuplicateCapability {
                    capability: descriptor.name.clone(),
                    existing_plugin: existing.plugin_id.clone(),
                    new_plugin: manifest.id.clone(),
                });
            }
            if staged.iter().any(|(k, _)| k == &key) {
                return Err(RegistryError::DuplicateCapability {
                    capability: descriptor.name.clone(),
                    existing_plugin: manifest.id.clone(),
                    new_plugin: manifest.id.clone(),
                });
            }
            let supported_version = policy.max_version(&descriptor.name);
            if descriptor.version > supported_version {
                refused.push(RefusedCapability {
                    plugin: manifest.id.clone(),
                    capability: descriptor.name.clone(),
                    advertised_version: descriptor.version,
                    supported_version,
                });
                continue;
            }
            if !schema_ids.contains(&descriptor.request_schema_id) {
                schema_ids.push(descriptor.request_schema_id.clone());
            }
            if !schema_ids.contains(&descriptor.response_schema_id) {
                schema_ids.push(descriptor.response_schema_id.clone());
            }
            staged.push((key, descriptor));
        }

        // Issue 3: one batched schema fetch for every id the listing named.
        let schemas = if schema_ids.is_empty() {
            BTreeMap::new()
        } else {
            plugin
                .schemas(&schema_ids)
                .map_err(|err| RegistryError::SchemaFetch {
                    plugin: manifest.id.clone(),
                    detail: err.to_string(),
                })?
        };

        // Compile both directions per staged capability.
        let mut ready: Vec<(String, Route)> = Vec::new();
        for (key, descriptor) in staged {
            let request = schemas.get(&descriptor.request_schema_id).ok_or_else(|| {
                RegistryError::SchemaFetch {
                    plugin: manifest.id.clone(),
                    detail: format!("schema id {} was not returned", descriptor.request_schema_id),
                }
            })?;
            let response = schemas.get(&descriptor.response_schema_id).ok_or_else(|| {
                RegistryError::SchemaFetch {
                    plugin: manifest.id.clone(),
                    detail: format!(
                        "schema id {} was not returned",
                        descriptor.response_schema_id
                    ),
                }
            })?;
            let request_value = parse_schema(&manifest.id, &descriptor.name, "request", request)?;
            let response_value =
                parse_schema(&manifest.id, &descriptor.name, "response", response)?;
            let request_validator = jsonschema::validator_for(&request_value).map_err(|err| {
                RegistryError::BadSchema {
                    plugin: manifest.id.clone(),
                    capability: descriptor.name.clone(),
                    detail: format!("request schema: {err}"),
                }
            })?;
            let response_validator = jsonschema::validator_for(&response_value).map_err(|err| {
                RegistryError::BadSchema {
                    plugin: manifest.id.clone(),
                    capability: descriptor.name.clone(),
                    detail: format!("response schema: {err}"),
                }
            })?;
            ready.push((
                key,
                Route {
                    plugin: Arc::clone(&plugin),
                    plugin_id: manifest.id.clone(),
                    capability: descriptor.name.clone(),
                    kind: descriptor.kind,
                    version: descriptor.version,
                    request_schema: request_value,
                    response_schema: response_value,
                    request_validator,
                    response_validator,
                },
            ));
        }

        // Commit.
        let instance = format!("{}#{}", manifest.id, self.next_instance);
        self.next_instance += 1;
        let capabilities: Vec<String> = ready
            .iter()
            .map(|(_, route)| route.capability.clone())
            .collect();
        for (key, route) in ready {
            self.routes.insert(key, route);
        }
        for refusal in &refused {
            let key = self.key_for(&refusal.plugin, &refusal.capability);
            self.refused_keys.insert(key, refusal.clone());
        }
        self.refused.extend(refused);
        self.plugins.insert(
            manifest.id.clone(),
            Entry {
                instance: instance.clone(),
                id: manifest.id.clone(),
                manifest,
                plugin,
                config: configuration,
                capabilities,
                enabled: true,
            },
        );
        Ok(instance)
    }

    /// Unregisters a plugin: shuts its instances down and drops the library.
    pub fn unregister(&mut self, plugin_id: &str) -> Result<(), RegistryError> {
        let Some(entry) = self.plugins.remove(plugin_id) else {
            return Err(RegistryError::UnknownPlugin {
                plugin: plugin_id.to_owned(),
            });
        };
        self.routes.retain(|_, route| route.plugin_id != plugin_id);
        self.refused_keys
            .retain(|_, refusal| refusal.plugin != plugin_id);
        self.refused.retain(|refusal| refusal.plugin != plugin_id);
        entry.plugin.shutdown().ok();
        Ok(())
    }

    /// Disables a plugin. It stays loaded, but none of its capabilities route.
    pub fn disable(&mut self, plugin_id: &str) -> Result<(), RegistryError> {
        let entry = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| RegistryError::UnknownPlugin {
                plugin: plugin_id.to_owned(),
            })?;
        entry.enabled = false;
        Ok(())
    }

    /// Re-enables a plugin.
    pub fn enable(&mut self, plugin_id: &str) -> Result<(), RegistryError> {
        let entry = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| RegistryError::UnknownPlugin {
                plugin: plugin_id.to_owned(),
            })?;
        entry.enabled = true;
        Ok(())
    }

    /// The configuration that was delivered to a plugin, if it is registered.
    pub fn configuration(&self, plugin_id: &str) -> Option<&Value> {
        self.plugins.get(plugin_id).map(|entry| &entry.config)
    }

    /// Every registered plugin, with its instance handle.
    pub fn list(&self) -> Vec<PluginInfo> {
        self.plugins
            .values()
            .map(|entry| PluginInfo {
                instance: entry.instance.clone(),
                id: entry.id.clone(),
                version: entry.manifest.version.clone(),
                abi_major: entry.manifest.abi_major,
                enabled: entry.enabled,
                capabilities: entry.capabilities.clone(),
            })
            .collect()
    }

    /// The capability names bare of any plugin prefix, in routing order.
    pub fn capability_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.routes.values().map(|r| r.capability.clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    /// The routing keys, in order.
    pub fn routing_keys(&self) -> Vec<String> {
        self.routes.keys().cloned().collect()
    }

    pub fn route(&self, key: &str) -> Option<&Route> {
        self.routes.get(key)
    }

    pub fn refused(&self) -> &[RefusedCapability] {
        &self.refused
    }

    pub fn len(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Routes one call by routing key.
    ///
    /// The request is validated before the plugin is called at all, and the
    /// response is validated inside the returned future, before the caller sees
    /// it.
    pub fn invoke<'a>(
        &'a self,
        key: &'a str,
        payload: Payload,
        call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>> {
        let Some(route) = self.routes.get(key) else {
            if let Some(refusal) = self.refused_keys.get(key) {
                let error = PluginError::VersionGated {
                    capability: refusal.capability.clone(),
                    advertised: refusal.advertised_version,
                    supported: refusal.supported_version,
                };
                return Box::pin(async move { Err(error) });
            }
            let capability = key.to_owned();
            return Box::pin(async move { Err(PluginError::CapabilityNotFound { capability }) });
        };

        if let Some(entry) = self.plugins.get(&route.plugin_id) {
            if !entry.enabled {
                let error = PluginError::Disabled {
                    plugin: route.plugin_id.clone(),
                    capability: route.capability.clone(),
                };
                return Box::pin(async move { Err(error) });
            }
        }

        if let Err(detail) = route.validate_request(&payload) {
            let error = PluginError::CallerPayloadInvalid {
                capability: route.capability.clone(),
                detail,
            };
            return Box::pin(async move { Err(error) });
        }

        Box::pin(async move {
            let response = route.plugin.invoke(&route.capability, payload, call).await?;
            route
                .validate_response(&response)
                .map_err(|detail| PluginError::ResponseViolation {
                    plugin: route.plugin_id.clone(),
                    capability: route.capability.clone(),
                    detail,
                })?;
            Ok(response)
        })
    }

    /// Cancels a pending request, routed by the key that started it.
    pub fn cancel(&self, key: &str, request_id: u64) -> Result<(), PluginError> {
        let route = self.routes.get(key).ok_or_else(|| {
            PluginError::CapabilityNotFound {
                capability: key.to_owned(),
            }
        })?;
        route.plugin.cancel(request_id)
    }

    /// Shuts every registered plugin down, in registration order.
    pub fn shutdown(&self) -> Vec<(String, Result<(), PluginError>)> {
        self.plugins
            .values()
            .map(|entry| (entry.id.clone(), entry.plugin.shutdown()))
            .collect()
    }
}

fn parse_schema(
    plugin: &str,
    capability: &str,
    which: &str,
    schema: &[u8],
) -> Result<Value, RegistryError> {
    serde_json::from_slice(schema).map_err(|err| RegistryError::BadSchema {
        plugin: plugin.to_owned(),
        capability: capability.to_owned(),
        detail: format!("{which} schema is not JSON: {err}"),
    })
}
