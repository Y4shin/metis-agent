//! The one asynchronous load entry: a bundle path in, a registered, serving
//! plugin out.
//!
//! One call composes every step the daemon would otherwise drive by hand. It
//! reads the bundle file, verifies its signature and chain, stages the archive
//! for this host's target, reads and validates the manifest, selects the
//! library the manifest declares for this host, maps it through the ABI gate,
//! delivers the configuration and the host context to the plugin's
//! asynchronous initialisation entry, waits for that future to resolve, checks
//! the manifest's capability list against what the library reports, and
//! registers the instance so a call routes through the registry.
//!
//! Three properties shape the design.
//!
//! **The caller's runtime is the only executor.** The entry is a plain
//! [`Future`]: it returns one and the caller polls it, so this crate still
//! starts no runtime and no thread, and the caller decides when plugin code
//! runs.
//!
//! **The failure names the first thing that was wrong.** The steps run in the
//! contract's order and each refusal is its own typed value, so a caller tells
//! an untrusted signature from an absent manifest from a plugin that refused
//! the configuration it was handed. The composed errors are the landed
//! [`BundleError`](crate::bundle::BundleError),
//! [`StageError`](crate::bundle::StageError) and
//! [`ManifestError`](crate::manifest::ManifestError) rather than restatements
//! of them, and the load-and-dispatch partition stays
//! [`CallError`](crate::CallError).
//!
//! **Initialisation failure has two classes, and they stay apart.** A plugin
//! that answers `invalid_argument` is telling the host that what it was handed
//! does not match what it declared, which is the configuration's class. Every
//! other terminal status is the plugin's own defect. The two are separate
//! values here rather than one status a caller has to interpret.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use plugin_contract::abi::MetisStatus;
use serde_json::{Value, json};

use crate::bundle::{BundleError, StageError, TrustStore, extract, verify_bundle};
use crate::manifest::{Manifest, ManifestError};
use crate::registry::{Registry, RegistryError};
use crate::{CallError, Host, Instance, Loaded};

/// What a bundle load can refuse.
///
/// The variants are the refusals a caller acts on differently, and they are in
/// the order the steps run: the file, the signature, the archive, the manifest,
/// the library and its table, the initialisation, and finally the registration.
/// A refusal names the first step that objected rather than a later consequence
/// of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The bundle file could not be read at all.
    ///
    /// It is the file's own refusal rather than the container's: a file that is
    /// not there, or one this process may not read, is a fact about the
    /// filesystem and not about a format this build cannot parse.
    Unreadable {
        /// The path that was read.
        path: PathBuf,
        /// What the filesystem said.
        detail: String,
    },
    /// The bundle is not one this host trusts: its signature, its chain, or the
    /// policy that accepts them refused it.
    Trust(BundleError),
    /// The bundle's archive could not be staged for this host's target.
    Stage(StageError),
    /// The manifest was absent, unreadable, invalid, or named another plugin.
    Manifest(ManifestError),
    /// The library could not be mapped, or its table was refused by the version
    /// gate.
    ///
    /// This is the load-and-dispatch partition's own error, kept as it is:
    /// nothing here restates it.
    Call(CallError),
    /// The plugin refused the configuration it was handed.
    ///
    /// Its status is `invalid_argument`, which is the one status that means the
    /// configuration does not match what the plugin declared rather than a
    /// defect of the plugin itself.
    ConfigurationRefused {
        /// The plugin's name, as its manifest declares it.
        plugin: String,
        /// The status the plugin's initialisation resolved with.
        status: MetisStatus,
    },
    /// The plugin's own initialisation failed, so the plugin is defective.
    ///
    /// Every terminal status other than `invalid_argument` is this class,
    /// because only `invalid_argument` speaks about what the host handed over.
    PluginDefect {
        /// The plugin's name, as its manifest declares it.
        plugin: String,
        /// The status the plugin's initialisation resolved with.
        status: MetisStatus,
    },
    /// The registry refused the instance's declaration.
    Registration(RegistryError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Unreadable { path, detail } => {
                write!(
                    f,
                    "the bundle at {} does not read: {detail}",
                    path.display()
                )
            }
            LoadError::Trust(error) => write!(f, "{error}"),
            LoadError::Stage(error) => write!(f, "{error}"),
            LoadError::Manifest(error) => write!(f, "{error}"),
            LoadError::Call(error) => write!(f, "{error}"),
            LoadError::ConfigurationRefused { plugin, status } => write!(
                f,
                "{plugin} refused the configuration it was handed ({})",
                status.as_str()
            ),
            LoadError::PluginDefect { plugin, status } => write!(
                f,
                "{plugin}'s initialisation failed, which is the plugin's own defect ({})",
                status.as_str()
            ),
            LoadError::Registration(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<BundleError> for LoadError {
    fn from(error: BundleError) -> Self {
        LoadError::Trust(error)
    }
}

impl From<StageError> for LoadError {
    fn from(error: StageError) -> Self {
        LoadError::Stage(error)
    }
}

impl From<ManifestError> for LoadError {
    fn from(error: ManifestError) -> Self {
        LoadError::Manifest(error)
    }
}

/// One loaded plugin: registered, initialised, and carrying what it declared.
pub struct Serving {
    name: String,
    instance: Arc<Instance>,
    capabilities: Vec<plugin_contract::capability::CapabilityDescriptor>,
    registry: Registry,
}

impl Serving {
    /// The instance name it is registered under, which is the manifest's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The instance itself.
    pub fn instance(&self) -> &Arc<Instance> {
        &self.instance
    }

    /// The capability descriptors that were registered.
    pub fn capabilities(&self) -> &[plugin_contract::capability::CapabilityDescriptor] {
        &self.capabilities
    }

    /// The registry the instance is registered in, which is what routes a call
    /// to it.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
}

/// The body field carrying the plugin's own configuration, fixed by contract
/// 18.
///
/// It is written here by hand rather than read out of `Initialisation`, whose
/// type lives behind `plugin-contract`'s `authoring` feature; that gate is off
/// for this crate, so the two names and the body's shape are kept in step with
/// the contract here.
const CONFIG_FIELD: &str = "config";

/// The body field carrying the host context, fixed by contract 18.
const HOST_CONTEXT_FIELD: &str = "host_context";

/// Loads one bundle into a registered, serving plugin.
///
/// `config` is the configuration the daemon resolved for this instance and
/// `host_context` is what the host tells the plugin about itself; both are
/// delivered as the two named parts of one initialisation payload. The runtime
/// does not interpret either: their value types belong to the daemon and to
/// `plugin-contract`'s authoring surface, and their contents are the plugin's
/// business.
///
/// The returned future is driven by the caller. Awaiting it runs every step,
/// including the plugin's asynchronous initialisation, on the caller's runtime.
pub async fn load_bundle(
    host: &Host,
    bundle_path: impl AsRef<Path>,
    config: &Value,
    host_context: &Value,
    trust: &TrustStore,
) -> Result<Serving, LoadError> {
    let path = bundle_path.as_ref();
    let bytes = std::fs::read(path).map_err(|error| LoadError::Unreadable {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    let verified = verify_bundle(&bytes, trust)?;
    let target = host_target();
    let staged = extract(&verified, &target)?;
    let manifest = Manifest::read(&staged, verified.leaf_common_name())?;
    let library = manifest.select_library(&staged, &target)?;
    // SAFETY: the library was mapped from a bundle whose signature and chain
    // this host's trust policy accepted, and extraction staged it from the
    // archive that signature covers, so it is code this host has decided to
    // trust.
    let loaded = Arc::new(unsafe { Loaded::open(&library, &manifest) }.map_err(LoadError::Call)?);
    let name = manifest.name().to_string();
    let body = initialisation_body(config, host_context);
    let init = loaded
        .init_plugin(host, &body)
        .map_err(|error| initialisation_refusal(&name, error))?;
    let instance = init
        .await
        .map_err(|error| initialisation_refusal(&name, error))?;
    let capabilities = instance.capabilities().to_vec();
    let mut registry = Registry::new();
    registry
        .register(&name, Arc::clone(&instance))
        .map_err(LoadError::Registration)?;
    Ok(Serving {
        name,
        instance,
        capabilities,
        registry,
    })
}

/// Classifies a failure of the initialisation hand-off.
///
/// A terminal status of `invalid_argument` is the plugin telling the host that
/// what it was handed does not match what it declared, which is the
/// configuration's class; every other terminal status is the plugin's own
/// defect. The entry refusing to start at all is classified by the same status,
/// because its refusal speaks about the same hand-off. Anything that is not a
/// terminal status, such as a library that could not be mapped or a capability
/// declaration that could not be read, stays the load-and-dispatch partition's
/// own refusal.
fn initialisation_refusal(plugin: &str, error: CallError) -> LoadError {
    match error {
        CallError::Init(status) | CallError::Status(status) => {
            if status == MetisStatus::InvalidArgument {
                LoadError::ConfigurationRefused {
                    plugin: plugin.to_string(),
                    status,
                }
            } else {
                LoadError::PluginDefect {
                    plugin: plugin.to_string(),
                    status,
                }
            }
        }
        other => LoadError::Call(other),
    }
}

/// This host's target, spelled the way contract 17.8 fixes it.
///
/// It is the operating system and the architecture joined by a hyphen, which is
/// what the extractor matches literally and what the manifest's `[libraries]`
/// table declares. A producer spells it the same way, so a second derivation
/// that disagreed would refuse a bundle carrying the library this host needs.
fn host_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The one initialisation body: a JSON object with the two named parts.
fn initialisation_body(config: &Value, host_context: &Value) -> Vec<u8> {
    json!({
        CONFIG_FIELD: config,
        HOST_CONTEXT_FIELD: host_context,
    })
    .to_string()
    .into_bytes()
}
