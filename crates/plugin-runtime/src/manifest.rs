//! The manifest: the bundle's own declaration of what it carries.
//!
//! A bundle's root carries `manifest.toml`, and this module turns that text
//! into a validated [`Manifest`] or a typed refusal. The text is the format an
//! author writes by hand, and it is documented as such in
//! `docs/plugin-contract.md`, so a plugin built without the SDK is packaged by
//! writing a file rather than by running a generator.
//!
//! This module holds three steps, in the order the checks run: reading the
//! manifest a staged bundle carries and checking the name it declares against
//! the name the bundle's chain carries, selecting this host's library from what
//! the manifest declares, and the manifest's ABI demand, which the mapping step
//! checks before it maps anything.
//!
//! Two properties shape the error surface. Every refusal is its own value, so a
//! caller tells an absent manifest from an invalid one, and a manifest that
//! declares nothing for this host from one whose declaration names a file the
//! bundle does not carry. And the errors name what is wrong rather than only
//! that something is, which is what makes a hand-written manifest fixable.

use std::collections::BTreeMap;
use std::path::PathBuf;

use plugin_contract::capability::CapabilityDescriptor;
use serde::Deserialize;
use serde_json::Value;

use crate::bundle::StagedBundle;

/// The manifest a bundle's root carries.
///
/// The fields are private and [`Manifest::parse`] is the only way to obtain a
/// value, so a `Manifest` is proof that the text it came from was the format
/// and that its identity is the one the bundle's chain carries.
///
/// It is `PartialEq` and not `Eq`, because a capability's schemas are JSON, and
/// a JSON value is only comparable for equality.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    name: String,
    version: String,
    abi_major: u32,
    min_abi_minor: u32,
    libraries: BTreeMap<String, String>,
    capabilities: Vec<CapabilityDescriptor>,
    config_schema: Value,
}

/// The manifest's text, as it is written.
///
/// It is a separate shape from [`Manifest`] on purpose: this one is the author's
/// spelling of the format, and `Manifest` is the validated value the rest of
/// the runtime passes around. A field this build does not know is ignored
/// rather than refused, because a manifest may carry something a newer producer
/// wrote without this build being unable to read it.
#[derive(Deserialize)]
struct ManifestText {
    name: String,
    version: String,
    abi_major: u32,
    min_abi_minor: u32,
    libraries: BTreeMap<String, String>,
    capabilities: Vec<CapabilityDescriptor>,
    config_schema: Value,
}

impl Manifest {
    /// Parses a manifest's text.
    ///
    /// The refusal names what is wrong with the text, which is what an author
    /// needs: the field that is missing, or the value that is not the declared
    /// type.
    pub fn parse(text: &str) -> Result<Manifest, ManifestError> {
        let text: ManifestText = toml::from_str(text).map_err(|error| ManifestError::Invalid {
            detail: error.to_string(),
        })?;
        Ok(Manifest {
            name: text.name,
            version: text.version,
            abi_major: text.abi_major,
            min_abi_minor: text.min_abi_minor,
            libraries: text.libraries,
            capabilities: text.capabilities,
            config_schema: text.config_schema,
        })
    }

    /// The plugin's name, which is also the name the bundle's chain carries.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The plugin's own version. It is recorded and never gates a load.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The ABI major the plugin is built for, which must be this build's.
    pub fn abi_major(&self) -> u32 {
        self.abi_major
    }

    /// The oldest ABI minor the plugin needs, which this build's known minor
    /// must not be lower than.
    pub fn min_abi_minor(&self) -> u32 {
        self.min_abi_minor
    }

    /// The library path this manifest declares for each target, relative to the
    /// archive root.
    pub fn libraries(&self) -> &BTreeMap<String, String> {
        &self.libraries
    }

    /// The capability list the host trusts for discovery, with each
    /// capability's inline schemas.
    pub fn capabilities(&self) -> &[CapabilityDescriptor] {
        &self.capabilities
    }

    /// The schema the daemon resolves this plugin's configuration against.
    pub fn config_schema(&self) -> &Value {
        &self.config_schema
    }

    /// The library this host loads, selected from the bundle the manifest came
    /// from.
    ///
    /// The manifest declares one library path per target, relative to the
    /// archive root, and the contract fixes that path as `lib/<target>/<file
    /// name>`. Selection resolves the declaration for `target` and answers
    /// where it is, or the refusal that it is not there.
    ///
    /// Only this target's declaration is resolved. A manifest that declares
    /// other targets, or none, is judged by what it declares for this host
    /// alone, so a bundle carrying several targets is not asked for the ones
    /// this host does not load.
    pub fn select_library(
        &self,
        staged: &StagedBundle,
        target: &str,
    ) -> Result<PathBuf, ManifestError> {
        let declared =
            self.libraries
                .get(target)
                .ok_or_else(|| ManifestError::NoTargetLibrary {
                    target: target.to_string(),
                })?;
        let selected = staged.root().join(declared);
        if !selected.is_file() {
            return Err(ManifestError::LibraryNotStaged {
                target: target.to_string(),
                path: selected,
            });
        }
        Ok(selected)
    }

    /// Reads the manifest a staged bundle carries, for the name the bundle's
    /// chain carries.
    ///
    /// The path is tested rather than assumed: extraction deliberately stages a
    /// bundle with no root manifest, so an absent manifest is this step's first
    /// refusal and not a file-not-found a caller has to interpret.
    pub fn read(staged: &StagedBundle, common_name: &str) -> Result<Manifest, ManifestError> {
        let path = staged.manifest_path();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ManifestError::Absent {
                    path: path.to_path_buf(),
                });
            }
            Err(error) => {
                return Err(ManifestError::Unreadable {
                    path: path.to_path_buf(),
                    detail: error.to_string(),
                });
            }
        };
        let manifest = Manifest::parse(&text)?;
        // The leaf-authorising certificate's common name is the plugin's name,
        // and the manifest is what says which plugin this is, so the two have to
        // agree: an identity the chain does not carry is a disagreement about
        // what was signed rather than a separate bundle.
        if manifest.name != common_name {
            return Err(ManifestError::NameMismatch {
                declared: manifest.name,
                chain: common_name.to_string(),
            });
        }
        Ok(manifest)
    }
}

/// What refusing a manifest can say.
///
/// The variants are the refusals a caller acts on differently: a bundle with no
/// manifest and a bundle with an unreadable one are defects of the file, an
/// invalid manifest is a defect of its text, and an identity that is not the
/// chain's is a disagreement about what the bundle says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    /// The bundle carries no manifest at the root.
    Absent {
        /// Where the manifest would have been staged.
        path: PathBuf,
    },
    /// The manifest is there and could not be read.
    Unreadable {
        /// The path that could not be read.
        path: PathBuf,
        /// What the filesystem said.
        detail: String,
    },
    /// The text is not the manifest format: a field is missing, a value is not
    /// the declared type, or the text is not TOML at all.
    Invalid {
        /// What is wrong with the text, naming the field where there is one.
        detail: String,
    },
    /// The manifest declares no library for this target.
    ///
    /// A bundle whose archive carries a library this host could stage and whose
    /// manifest does not declare one is a disagreement about what is inside it,
    /// which is why this refusal is the manifest's and not the extractor's.
    NoTargetLibrary {
        /// The target this host asked for.
        target: String,
    },
    /// The manifest declares a library for this target that the bundle does not
    /// carry there.
    LibraryNotStaged {
        /// The target this host asked for.
        target: String,
        /// The path the declaration names, which is not a file.
        path: PathBuf,
    },
    /// The manifest declares a name that is not the name the bundle's
    /// leaf-authorising certificate carries.
    NameMismatch {
        /// The name the manifest declares.
        declared: String,
        /// The name the chain carries.
        chain: String,
    },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Absent { path } => {
                write!(f, "the bundle carries no manifest at {}", path.display())
            }
            ManifestError::Unreadable { path, detail } => write!(
                f,
                "the manifest at {} does not read: {detail}",
                path.display()
            ),
            ManifestError::Invalid { detail } => {
                write!(
                    f,
                    "the manifest is not the format an author writes: {detail}"
                )
            }
            ManifestError::NoTargetLibrary { target } => {
                write!(
                    f,
                    "the manifest declares no library for the target {target}"
                )
            }
            ManifestError::LibraryNotStaged { target, path } => write!(
                f,
                "the manifest declares the library {target} at {}, and the bundle does not carry it",
                path.display()
            ),
            ManifestError::NameMismatch { declared, chain } => write!(
                f,
                "the manifest names the plugin {declared}, and the bundle's chain names {chain}"
            ),
        }
    }
}

impl std::error::Error for ManifestError {}
