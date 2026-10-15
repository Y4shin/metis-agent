//! Start-up policy for a directory of bundles.
//!
//! One call reads a plugin directory, loads every bundle it holds, and answers
//! either a report of what started and what start-up disabled, or the single
//! outcome that stopped start-up. The policy itself is a table, documented as
//! the rule in `docs/plugin-contract.md` section 19, and this module is that
//! table read at one seam.
//!
//! Three properties shape the design.
//!
//! **The policy is one classification, not a second load path.** Every bundle
//! goes through the landed [`load_bundle`], and the outcome it answers is
//! classified by variant: a trust failure, a bundle file that cannot be read, an
//! absent or invalid manifest and a refused unload stop start-up, and everything
//! else disables that one plugin and leaves the daemon running. Nothing here
//! re-drives the steps a load already ran.
//!
//! **A disabled plugin is still listable.** It appears in the report with its
//! name, the refusal its load answered, and the capabilities its manifest
//! declared, and it answers the disabled route error carrying that reason rather
//! than disappearing into "never loaded".
//!
//! **A disabled plugin is not offered.** What an agent would be offered is the
//! serving plugins' declarations and nothing else, because a disabled plugin
//! cannot serve what it declares.

use std::path::{Path, PathBuf};

use plugin_contract::capability::CapabilityDescriptor;
use serde_json::{Map, Value, json};

use crate::Host;
use crate::bundle::{TrustStore, extract, verify_bundle};
use crate::load::{LoadError, Serving, host_target, load_bundle};
use crate::manifest::{Manifest, ManifestError};
use crate::registry::RouteError;

/// The host-context field naming the plugin's instance, fixed by contract 18.
const INSTANCE_NAME_FIELD: &str = "instance_name";

/// The host-context field carrying the capability version the host serves,
/// fixed by contract 18.
const CAPABILITY_VERSION_FIELD: &str = "capability_version";

/// The capability descriptor version this build serves.
const CAPABILITY_VERSION: u32 = 1;

/// Starts every bundle a plugin directory holds.
///
/// The directory is the whole discovery rule: every regular file directly in it
/// is one bundle, named by its file's stem, and nothing is scanned recursively
/// and no discovery file is read.
///
/// `config` is the per-plugin configuration the caller resolved, keyed by the
/// plugin's name, which is the name the policy knows the bundle by before its
/// manifest is read: the file stem. A bundle with no entry starts with no
/// configuration. The host context is built here from that same name, because
/// the policy builds the host the load takes and no caller can make two plugins
/// claim one name.
///
/// The returned report carries what started and what was disabled. The outcome
/// that stops start-up is returned instead of a report, because a daemon that
/// cannot describe its plugin set must not continue with part of it, and it
/// carries the plugins this pass had already started, so the caller can release
/// them: the runtime unloads nothing on its own and holds no deadline of its
/// own.
pub async fn start_plugins(
    dir: impl AsRef<Path>,
    config: &Map<String, Value>,
    trust: &TrustStore,
) -> Result<StartupReport, StartupStop> {
    let dir = dir.as_ref();
    let files = bundles(dir).map_err(|error| StartupStop {
        path: dir.to_path_buf(),
        error,
        serving: Vec::new(),
    })?;
    let host = Host::new();
    let mut serving: Vec<Serving> = Vec::new();
    let mut disabled: Vec<DisabledPlugin> = Vec::new();
    for path in files {
        let name = bundle_name(&path);
        let plugin_config = config.get(&name).cloned().unwrap_or(Value::Null);
        let context = host_context(&name);
        match load_bundle(&host, &path, &plugin_config, &context, trust).await {
            Ok(started) => serving.push(started),
            // The plugins this pass already started stop with it: they are handed
            // to the caller rather than unloaded here, because the caller owns
            // the deadline and this pass has none.
            Err(error) if stops_startup(&error) => {
                return Err(StartupStop {
                    path,
                    error,
                    serving,
                });
            }
            Err(error) => {
                let placeholder = match describe(&path, trust) {
                    Some((declared, capabilities)) => DisabledPlugin {
                        name: declared,
                        reason: error,
                        capabilities,
                    },
                    None => DisabledPlugin {
                        name,
                        reason: error,
                        capabilities: Vec::new(),
                    },
                };
                disabled.push(placeholder);
            }
        }
    }
    Ok(StartupReport {
        host,
        serving,
        disabled,
    })
}

/// What one start-up pass produced.
pub struct StartupReport {
    host: Host,
    serving: Vec<Serving>,
    disabled: Vec<DisabledPlugin>,
}

impl StartupReport {
    /// The host every bundle of this pass was loaded through.
    ///
    /// It is the policy's own, because a load delivers its callbacks to the host
    /// it is given and this pass has no caller-supplied one. What started is
    /// read from here: the recorded callbacks and the ordering ledger.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The plugins that started, in the order the directory names their files.
    pub fn serving(&self) -> &[Serving] {
        &self.serving
    }

    /// The plugins start-up disabled, in the same order.
    ///
    /// Each carries the name it is listed under, the refusal its load answered,
    /// and the capabilities its manifest declared. A placeholder is listable
    /// rather than absent, so a caller can tell a plugin that was turned off
    /// from one that was never configured.
    pub fn disabled(&self) -> &[DisabledPlugin] {
        &self.disabled
    }

    /// Every capability an agent would be offered, qualified the way a call
    /// names it.
    ///
    /// It is the serving plugins' declarations and nothing else: a disabled
    /// plugin's capabilities are known, listed on its placeholder, and not
    /// offered, because a plugin that was disabled cannot serve what it
    /// declares.
    pub fn offered_capabilities(&self) -> Vec<String> {
        let mut offered = Vec::new();
        for serving in &self.serving {
            for capability in serving.capabilities() {
                offered.push(format!("{}.{}", serving.name(), capability.name));
            }
        }
        offered
    }

    /// Invokes one capability, the way a daemon routes a call.
    ///
    /// A capability a serving plugin declares is called through that plugin's own
    /// registry, which stays the one routing path, and a capability a disabled
    /// plugin declares is answered as the disabled route error carrying the
    /// reason its load refused. A name nothing declares is unknown, so a caller
    /// still tells the three states apart.
    pub async fn call(
        &self,
        qualified: &str,
        body: &[u8],
        request_id: u64,
    ) -> Result<Vec<u8>, RouteError> {
        if let Some(disabled) = self.disabled_capability(qualified) {
            return Err(RouteError::Disabled {
                name: qualified.to_string(),
                reason: disabled.reason().to_string(),
            });
        }
        for serving in &self.serving {
            match serving.registry().call(qualified, body, request_id).await {
                Err(RouteError::Unknown(_)) => continue,
                outcome => return outcome,
            }
        }
        Err(RouteError::Unknown(qualified.to_string()))
    }

    /// The disabled plugin that declares a qualified capability name.
    ///
    /// The name is matched exactly, as `<plugin>.<capability>`: the plugin's own
    /// name may carry a dot, so the split is by the prefix the placeholder's name
    /// matches rather than by the first dot.
    fn disabled_capability(&self, qualified: &str) -> Option<&DisabledPlugin> {
        self.disabled.iter().find(|plugin| {
            qualified
                .strip_prefix(plugin.name())
                .and_then(|rest| rest.strip_prefix('.'))
                .is_some_and(|tool| {
                    plugin
                        .capabilities()
                        .iter()
                        .any(|capability| capability.name == tool)
                })
        })
    }
}

/// One plugin start-up disabled, and why.
pub struct DisabledPlugin {
    name: String,
    reason: LoadError,
    capabilities: Vec<CapabilityDescriptor>,
}

impl DisabledPlugin {
    /// The plugin's name: what its manifest declares, or the bundle file's stem
    /// when no manifest was readable.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Why start-up disabled it, which is the refusal its load answered.
    ///
    /// It is the load's own typed error rather than a restatement of it, so a
    /// caller reads the class of the refusal the same way a stop is read.
    pub fn reason(&self) -> &LoadError {
        &self.reason
    }

    /// The capabilities the manifest declared, which this plugin does not
    /// serve.
    ///
    /// The list is empty when no manifest was readable: a plugin whose
    /// declaration was never read declares nothing.
    pub fn capabilities(&self) -> &[CapabilityDescriptor] {
        &self.capabilities
    }
}

/// Why start-up stopped, and what it had started by then.
///
/// A pass that stops holds the plugins it already started, because the runtime
/// unloads nothing on its own: the caller owns the release and the deadline it
/// is given. A daemon that stops releases them before it does anything else.
#[derive(Debug)]
pub struct StartupStop {
    path: PathBuf,
    error: LoadError,
    serving: Vec<Serving>,
}

impl StartupStop {
    /// The bundle that stopped it, or the directory that could not be listed.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The outcome that stopped it.
    pub fn error(&self) -> &LoadError {
        &self.error
    }

    /// The plugins this pass had already started, in the order they started.
    ///
    /// They are handed over rather than released here, because the deadline
    /// belongs to the caller: a pass has no deadline of its own and the runtime
    /// holds no timer, so [`Serving::release`](crate::load::Serving::release) is
    /// the caller's to run. Nothing is unloaded on this path.
    pub fn serving(&self) -> &[Serving] {
        &self.serving
    }

    /// The plugins this pass had already started, taken so the caller can
    /// release them.
    ///
    /// It is the owned form of [`StartupStop::serving`], for the one thing the
    /// caller does with them: `release` consumes a `Serving`.
    pub fn into_serving(self) -> Vec<Serving> {
        self.serving
    }
}

impl std::fmt::Display for StartupStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "start-up stopped at {}: {}",
            self.path.display(),
            self.error
        )
    }
}

impl std::error::Error for StartupStop {}

/// The bundle files a plugin directory holds, in the order a listing names them.
///
/// Only regular files directly in the directory are bundles: a directory inside
/// it is not one, and nothing here walks into one.
fn bundles(dir: &Path) -> Result<Vec<PathBuf>, LoadError> {
    let unreadable = |detail: String| LoadError::Unreadable {
        path: dir.to_path_buf(),
        detail,
    };
    let entries = std::fs::read_dir(dir).map_err(|error| unreadable(error.to_string()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| unreadable(error.to_string()))?;
        let path = entry.path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// The name a bundle is known by before its manifest is read: its file's stem.
///
/// It is the name the configuration is resolved for, the name the host context
/// carries, and the name a placeholder falls back to when no manifest was
/// readable, so a file named `broken.metisbnd` is named `broken`.
fn bundle_name(path: &Path) -> String {
    match path.file_stem().or_else(|| path.file_name()) {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

/// The host context this policy builds for a plugin.
///
/// The policy builds it rather than taking it, so the name a plugin is
/// initialised as is the name the policy knows the bundle by. `data_dir` is
/// absent, because a plugin directory pass resolves no data directory, and the
/// capability version is the one this build serves.
fn host_context(name: &str) -> Value {
    json!({
        INSTANCE_NAME_FIELD: name,
        CAPABILITY_VERSION_FIELD: CAPABILITY_VERSION,
    })
}

/// The name and capabilities a bundle declares, read a second time.
///
/// A load that refused hands back no `Manifest`, so the placeholder path reads
/// the bundle again: it verifies the bytes, stages the archive for this host and
/// reads the manifest. Any refusal along the way answers `None`, which is the
/// case of a bundle whose declaration was never readable, such as one whose
/// container version this build does not know: the placeholder then carries the
/// file's name and no capabilities.
///
/// The second read is the cost of leaving the load entry's landed surface as it
/// is, and it runs only for a bundle that was disabled, never for one that
/// started.
fn describe(path: &Path, trust: &TrustStore) -> Option<(String, Vec<CapabilityDescriptor>)> {
    let bytes = std::fs::read(path).ok()?;
    let verified = verify_bundle(&bytes, trust).ok()?;
    let staged = extract(&verified, &host_target()).ok()?;
    let manifest = Manifest::read(&staged, verified.leaf_common_name()).ok()?;
    Some((
        manifest.name().to_string(),
        manifest.capabilities().to_vec(),
    ))
}

/// Whether a load outcome stops start-up rather than disabling one plugin.
///
/// The split is one predicate and not a variant match where the bundle is
/// concerned: `BundleError::is_trust_failure` answers whether the file claims a
/// signature this host does not trust, which is evidence something is wrong and
/// stops start-up, while a container this build cannot read only means one
/// bundle does not work here. A bundle file that cannot be read at all, a
/// manifest that is absent, unreadable, invalid or not the name the bundle's
/// chain carries, and an unload that refused are the other stops.
fn stops_startup(error: &LoadError) -> bool {
    match error {
        LoadError::Trust(failure) => failure.is_trust_failure(),
        LoadError::Unreadable { .. } | LoadError::UnloadRefused { .. } => true,
        LoadError::Manifest(
            ManifestError::Absent { .. }
            | ManifestError::Unreadable { .. }
            | ManifestError::Invalid { .. }
            | ManifestError::NameMismatch { .. },
        ) => true,
        // The manifest's two refusals about this host's library are the manifest
        // side of the extractor's two, so all four are one disable reason with
        // four names: a bundle that carries no library this host can use is a
        // bundle that does not work here, not evidence that something is wrong.
        LoadError::Manifest(
            ManifestError::NoTargetLibrary { .. } | ManifestError::LibraryNotStaged { .. },
        )
        | LoadError::Stage(_)
        | LoadError::Call(_)
        | LoadError::CapabilityMismatch { .. }
        | LoadError::Registration(_) => false,
        // A plugin whose initialisation failed is the same disable with a name
        // worth reading: it started, refused, and was unloaded before this pass
        // listed it as a placeholder.
        LoadError::ConfigurationRefused { .. } | LoadError::PluginDefect { .. } => false,
    }
}
