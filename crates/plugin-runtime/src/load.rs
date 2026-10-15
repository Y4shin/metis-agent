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

use std::future::poll_fn;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::Instant;

use plugin_contract::abi::MetisStatus;
use plugin_contract::capability::CapabilityDescriptor;
use serde_json::{Value, json};

use crate::bundle::{BundleError, StageError, TrustStore, extract, verify_bundle};
use crate::manifest::{Manifest, ManifestError};
use crate::registry::{Registry, RegistryError};
use crate::{CallError, CancelReport, Host, Instance, Loaded};

/// What a bundle load can refuse.
///
/// The variants are the refusals a caller acts on differently, and most of them
/// are in the order the steps run: the file, the signature, the archive, the
/// manifest, the library and its table, the initialisation, the capability
/// cross-check, and finally the registration. A refusal names the first step
/// that objected rather than a later consequence of it.
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
    /// The manifest's capability list and the library's own declaration
    /// disagree.
    ///
    /// The manifest is what a host trusts for discovery and the library's
    /// declaration is the cross-check, so the two have to carry the same
    /// capabilities. The refusal names the capability that differed, because a
    /// manifest generated from the same source as the plugin should never
    /// disagree with it and which side is wrong decides what the author fixes.
    CapabilityMismatch {
        /// The capability that differed, spelled as the declaration that
        /// carries it spells it.
        capability: String,
        /// What the disagreement is, naming the declaration the capability is
        /// missing from.
        detail: String,
    },
    /// The registry refused the instance's declaration.
    Registration(RegistryError),
    /// A plugin whose initialisation failed could not be unloaded, so start-up
    /// has to stop.
    ///
    /// It is the recovery's own refusal and not the initialisation's: the
    /// plugin's future was dropped and the instance pointer the entry handed out
    /// was then passed to `destroy_instance`, which refused. Continuing would
    /// leave the daemon holding an instance it cannot describe, so this is the
    /// one cleanup failure a caller must act on rather than record.
    UnloadRefused {
        /// The plugin's name, as its manifest declares it.
        plugin: String,
        /// The teardown entry that refused, named as the table names it.
        step: String,
        /// The status that entry answered.
        status: MetisStatus,
    },
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
            LoadError::CapabilityMismatch { capability, detail } => write!(
                f,
                "the manifest and the library disagree about the capability list: {capability} {detail}"
            ),
            LoadError::Registration(error) => write!(f, "{error}"),
            LoadError::UnloadRefused {
                plugin,
                step,
                status,
            } => write!(
                f,
                "{plugin} could not be unloaded, because {step} answered {}",
                status.as_str()
            ),
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
    capabilities: Vec<CapabilityDescriptor>,
    registry: Registry,
}

impl std::fmt::Debug for Serving {
    /// The instance is named rather than printed, because what a reader of a
    /// report needs is which plugin this is and what it declared. A start-up
    /// stop carries the servings it started, and its refusal is printed with
    /// them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Serving")
            .field("name", &self.name)
            .field("capabilities", &self.capabilities.len())
            .finish_non_exhaustive()
    }
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
    pub fn capabilities(&self) -> &[CapabilityDescriptor] {
        &self.capabilities
    }

    /// The registry the instance is registered in, which is what routes a call
    /// to it.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The registry the instance is registered in, mutably.
    ///
    /// It is the path to turning this one entry off and on again: a disabled
    /// entry refuses a new call and leaves the instance, its in-flight calls and
    /// its library alone, so disable and enable are registry state rather than a
    /// lifecycle step. The release entry is still the normal way an instance
    /// leaves the registry.
    pub fn registry_mut(&mut self) -> &mut Registry {
        &mut self.registry
    }

    /// Releases the instance, bounds the wait by the caller's deadline, and
    /// reports what it did.
    ///
    /// One call performs the contract's teardown order: the instance leaves the
    /// registry, the calls it still holds are cancelled, the caller's own drops
    /// are waited for up to `deadline`, the plugin's shutdown entry runs, the
    /// instance is destroyed, and the library is unloaded. Every step and every
    /// refusal is recorded on the ordering ledger, which the report carries.
    ///
    /// The deadline is the same [`Instant`] the landed `Instance::cancel_all`
    /// takes, and the runtime arms no timer of its own: it is named back in the
    /// report and it bounds the wait, and no second deadline type is introduced.
    ///
    /// An instance that cannot be released is not forced. A quarantined
    /// instance, a call still in flight at the deadline, a shutdown entry that
    /// answered a status other than `ok`, and an unload the landed guards
    /// refuse all leave the library mapped, and the report keeps the library
    /// reference that keeps it mapped so the caller can read the refusal.
    ///
    /// The returned future is driven by the caller, like every other future this
    /// crate hands back. The runtime holds no handle on the calls the caller
    /// started, so the caller's own drop is what drains them: this entry cancels
    /// them and waits.
    pub async fn release(self, deadline: Instant) -> ReleaseReport {
        let Serving {
            name,
            instance,
            capabilities: _,
            mut registry,
        } = self;
        let host = instance.host().clone();
        let from = host.ordering().len();

        // The instance leaves the routing table first, so nothing new can reach
        // it, and the reference the registry held goes with the entry.
        let entry = registry.unregister(&name);
        host.order(format!("instance {name} unregistered from its registry"));
        drop(entry);

        // Cancel or drain to the caller's deadline. The plugin gets the chance
        // the contract gives it to resolve each call as cancelled; the caller's
        // own drop is what removes the future.
        let cancelled = instance.cancel_all(deadline);
        if !cancelled.cancelled.is_empty() {
            host.order(format!(
                "release cancelled {} call(s) the instance still held: {:?}",
                cancelled.cancelled.len(),
                cancelled.cancelled
            ));
        }
        while !instance.in_flight().is_empty() && Instant::now() < deadline {
            yield_now().await;
        }

        let mut in_flight: Vec<u64> = instance.in_flight().into_iter().collect();
        in_flight.sort_unstable();
        // The library stays mapped until this reference is dropped, so an
        // instance that cannot be released keeps it and a release that succeeds
        // drops it: an unload the guards refuse cannot unmap code that may still
        // be resident by the contract's own rule.
        let mut library = Some(Arc::clone(instance.loaded()));
        let mut refusal: Option<ReleaseRefusal> = None;
        if instance.is_quarantined() {
            // A poll this instance abandoned may still be running, so nothing
            // here may unmap its library.
            host.order("release refused: the instance is quarantined");
            refusal = Some(ReleaseRefusal::Quarantined);
        } else if !in_flight.is_empty() {
            host.order(format!(
                "release refused: {} call(s) were still in flight at the deadline: {in_flight:?}",
                in_flight.len()
            ));
            refusal = Some(ReleaseRefusal::FuturesInFlight(in_flight));
        } else {
            match instance.shutdown() {
                Err(error) => {
                    host.order(format!(
                        "release refused: the shutdown entry answered {error}"
                    ));
                    refusal = Some(ReleaseRefusal::Shutdown(error));
                }
                Ok(()) => match instance.unload() {
                    Ok(()) => {
                        if let Some(last) = library.take() {
                            let path = last.path().to_path_buf();
                            drop(last);
                            host.order(format!("library unloaded: {}", path.display()));
                        }
                    }
                    Err(error) => {
                        // The guard's own line is already on the ledger; this is
                        // the release's record of the step it refused.
                        host.order(format!("release refused: the unload refused: {error}"));
                        refusal = Some(ReleaseRefusal::Unload(error));
                    }
                },
            }
        }
        let ledger = host.ordering()[from..].to_vec();
        ReleaseReport {
            name,
            deadline,
            cancelled,
            refusal,
            resident: library,
            ledger,
        }
    }
}

/// Why an instance could not be released.
///
/// Each variant is one refusal the contract names: a quarantined instance whose
/// abandoned poll never returns, calls still in flight when the caller's
/// deadline passed, a shutdown entry that answered a status other than `ok`, and
/// an unload the landed guards refused. A release that answers one of these
/// leaves the library mapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseRefusal {
    /// A poll this instance abandoned is still running, so its library stays
    /// resident.
    Quarantined,
    /// The caller had not dropped these calls when the deadline passed, so the
    /// instance could not quiesce.
    FuturesInFlight(Vec<u64>),
    /// The plugin's shutdown entry answered a status other than `ok`.
    Shutdown(CallError),
    /// The unload refused, and this is the landed guard's own error.
    Unload(CallError),
}

impl ReleaseRefusal {
    /// The teardown step that refused, named as the sequence names it.
    ///
    /// A quarantined instance refuses the unload, because nothing may unmap a
    /// library a poll may still be running; calls still in flight refuse the
    /// drain, because the instance cannot quiesce while they are alive.
    pub fn step(&self) -> &'static str {
        match self {
            ReleaseRefusal::Quarantined | ReleaseRefusal::Unload(_) => "unload",
            ReleaseRefusal::FuturesInFlight(_) => "drain",
            ReleaseRefusal::Shutdown(_) => "shutdown",
        }
    }
}

impl std::fmt::Display for ReleaseRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReleaseRefusal::Quarantined => write!(
                f,
                "a poll this instance abandoned may still be running, so its library stays resident"
            ),
            ReleaseRefusal::FuturesInFlight(ids) => write!(
                f,
                "{} call(s) were still in flight at the deadline: {ids:?}",
                ids.len()
            ),
            ReleaseRefusal::Shutdown(error) => {
                write!(f, "the shutdown entry answered {error}")
            }
            ReleaseRefusal::Unload(error) => write!(f, "the unload refused: {error}"),
        }
    }
}

impl std::error::Error for ReleaseRefusal {}

/// What one release entry did, and what it added to the ordering ledger.
///
/// It is the caller's record of one instance: the name it was released under,
/// the deadline it was given, the cancellation pass it ran, the ledger entries
/// it added, and, when the instance could not be released, the refusal and the
/// library reference that keeps the library mapped.
///
/// The ledger entries are the host's shared ordering from this release's own
/// start index, not a slice the host can prove belongs to this instance alone:
/// the ordering carries no instance attribution, so a second release running
/// concurrently on the same host can contribute lines to a report. The slice is
/// exact on a host that runs one release at a time, which is what every test of
/// this entry does, and a daemon that releases several instances on one host
/// must not read it as one instance's own record.
pub struct ReleaseReport {
    name: String,
    deadline: Instant,
    cancelled: CancelReport,
    refusal: Option<ReleaseRefusal>,
    resident: Option<Arc<Loaded>>,
    ledger: Vec<String>,
}

impl ReleaseReport {
    /// The instance name the release was for, which is what a shutdown report
    /// names.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The deadline the caller gave, named back rather than armed.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The cancellation pass the release ran over the calls it found.
    pub fn cancelled(&self) -> &CancelReport {
        &self.cancelled
    }

    /// Whether the instance was released, which means every future was gone, the
    /// plugin quiesced, the instance was destroyed and the library was unloaded.
    pub fn released(&self) -> bool {
        self.refusal.is_none()
    }

    /// Why the instance could not be released, or `None` when it was.
    pub fn refusal(&self) -> Option<&ReleaseRefusal> {
        self.refusal.as_ref()
    }

    /// The library reference this release kept, so an instance that could not be
    /// released stays mapped.
    ///
    /// It is `None` exactly when the instance was released: a release that did
    /// not unload the library drops this reference and lets the platform unmap
    /// it, and a release that refused keeps it so the library stays resident.
    pub fn resident(&self) -> Option<&Arc<Loaded>> {
        self.resident.as_ref()
    }

    /// The ordering-ledger entries this release added, in the order they were
    /// added.
    ///
    /// The host keeps one ordering for every instance it serves and records no
    /// instance attribution, so this is the host's ordering from this release's
    /// own start index: a second release running concurrently on the same host
    /// can appear inside this slice. It is exact for a host that runs one
    /// release at a time, and a caller that releases several instances on one
    /// host reads [`ReleaseReport::name`] and each line's own wording rather
    /// than treating the slice as one instance's.
    pub fn ledger(&self) -> &[String] {
        &self.ledger
    }
}

impl std::fmt::Display for ReleaseReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.refusal {
            None => write!(f, "{} was released", self.name),
            Some(refusal) => write!(
                f,
                "{} could not be released at the {} step: {refusal}",
                self.name,
                refusal.step()
            ),
        }
    }
}

/// Yields once to the caller's runtime.
///
/// The runtime owns no timer, so "wait until the deadline" is a yield the
/// caller's runtime schedules: the future wakes itself and returns `Pending`
/// once per round, so a single-threaded caller runs other work instead of
/// spinning on a loop that awaits nothing. The deadline itself is read in the
/// loop's condition, which is what ends the wait.
async fn yield_now() {
    let mut yielded = false;
    poll_fn(move |context| {
        if yielded {
            return Poll::Ready(());
        }
        yielded = true;
        context.waker().wake_by_ref();
        Poll::Pending
    })
    .await
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
    // The instance's own view of the host: it names the instance on every
    // callback, so a sink installed on the host the caller passed hears which
    // plugin spoke. The name is the manifest's, which is the name the instance
    // is registered under.
    let instance_host = host.for_instance(&name);
    let init = loaded
        .init_plugin(&instance_host, &body)
        .map_err(|error| initialisation_refusal(&name, error))?;
    let outcome = init.await;
    // The initialisation future has been dropped by the end of the statement
    // above, so a teardown step of that recovery which refused is recorded by
    // now. It is read before the failure is classified, because it is the more
    // severe fact: a plugin that could not be unloaded is not the same outcome as
    // one that failed and was unloaded cleanly, and only the caller can decide to
    // stop.
    if let Some(refused) = host.take_refused_unload() {
        return Err(LoadError::UnloadRefused {
            plugin: name,
            step: refused.step.to_string(),
            status: refused.status,
        });
    }
    let instance = outcome.map_err(|error| initialisation_refusal(&name, error))?;
    cross_check(&manifest, &instance)?;
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

/// Cross-checks the manifest's capability list against what the library
/// reports.
///
/// The two declarations have to carry the same capability names: the manifest
/// is what a host trusts for discovery and the library's declaration is the
/// cross-check, so a name on one side and not the other is a disagreement about
/// what this bundle is. Each declaration is walked in its own order and the
/// first difference is the one refused, so the refusal names a capability a
/// reader can find rather than an arbitrary member of a set.
///
/// Only the names are compared. The manifest's schemas are declared text that
/// an author writes, and the library derives its own, so comparing them would
/// refuse a bundle the format is meant to allow. A capability's kind and
/// version are gated per capability where a call routes, not here.
fn cross_check(manifest: &Manifest, instance: &Instance) -> Result<(), LoadError> {
    let reported = |name: &str| {
        instance
            .capabilities()
            .iter()
            .any(|capability| capability.name == name)
    };
    for capability in manifest.capabilities() {
        if !reported(&capability.name) {
            return Err(LoadError::CapabilityMismatch {
                capability: capability.name.clone(),
                detail: "is declared by the manifest and not by the library".to_string(),
            });
        }
    }
    let declared = |name: &str| {
        manifest
            .capabilities()
            .iter()
            .any(|capability| capability.name == name)
    };
    for capability in instance.capabilities() {
        if !declared(&capability.name) {
            return Err(LoadError::CapabilityMismatch {
                capability: capability.name.clone(),
                detail: "is declared by the library and not by the manifest".to_string(),
            });
        }
    }
    Ok(())
}

/// This host's target, spelled the way contract 17.8 fixes it.
///
/// It is the operating system and the architecture joined by a hyphen, which is
/// what the extractor matches literally and what the manifest's `[libraries]`
/// table declares. A producer spells it the same way, so a second derivation
/// that disagreed would refuse a bundle carrying the library this host needs.
/// The start-up policy reads the same string when it re-reads a bundle for a
/// disabled plugin's placeholder, which is why it is crate-visible rather than
/// private to this module.
pub(crate) fn host_target() -> String {
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
