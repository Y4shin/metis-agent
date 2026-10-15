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

use serde_json::{Map, Value, json};

use crate::Host;
use crate::bundle::TrustStore;
use crate::load::{LoadError, Serving, load_bundle};

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
/// cannot describe its plugin set must not continue with part of it.
pub async fn start_plugins(
    dir: impl AsRef<Path>,
    config: &Map<String, Value>,
    trust: &TrustStore,
) -> Result<StartupReport, StartupStop> {
    let dir = dir.as_ref();
    let files = bundles(dir).map_err(|error| StartupStop {
        path: dir.to_path_buf(),
        error,
    })?;
    let host = Host::new();
    let mut serving: Vec<Serving> = Vec::new();
    for path in files {
        let name = bundle_name(&path);
        let plugin_config = config.get(&name).cloned().unwrap_or(Value::Null);
        let context = host_context(&name);
        match load_bundle(&host, &path, &plugin_config, &context, trust).await {
            Ok(started) => serving.push(started),
            Err(error) if stops_startup(&error) => return Err(StartupStop { path, error }),
            Err(_error) => todo!("a disable outcome, added by the next cycle"),
        }
    }
    Ok(StartupReport { host, serving })
}

/// What one start-up pass produced.
pub struct StartupReport {
    host: Host,
    serving: Vec<Serving>,
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
}

/// Why start-up stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupStop {
    path: PathBuf,
    error: LoadError,
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

/// Whether a load outcome stops start-up rather than disabling one plugin.
///
/// The split is one predicate and not a variant match where the bundle is
/// concerned: `BundleError::is_trust_failure` answers whether the file claims a
/// signature this host does not trust, which is evidence something is wrong and
/// stops start-up, while a container this build cannot read only means one
/// bundle does not work here. A bundle file that cannot be read at all, a
/// manifest that is absent or invalid, and an unload that refused are the other
/// three stops.
fn stops_startup(error: &LoadError) -> bool {
    match error {
        LoadError::Trust(failure) => failure.is_trust_failure(),
        LoadError::Unreadable { .. } | LoadError::Manifest(_) => true,
        LoadError::Stage(_)
        | LoadError::Call(_)
        | LoadError::ConfigurationRefused { .. }
        | LoadError::PluginDefect { .. }
        | LoadError::CapabilityMismatch { .. }
        | LoadError::Registration(_) => false,
    }
}
