//! The archive extractor: a verified bundle's archive, staged for this host.
//!
//! Verification has already accepted the bundle's bytes. This module's job is
//! the layer between those bytes and a load: write the root manifest and this
//! host's target directory into a staging directory of this process's own, and
//! answer a typed refusal for anything the contract's rules refuse.
//!
//! Two properties shape it.
//!
//! **Nothing is left behind.** The staging directory is created before the
//! archive is read and removed before a refusal is answered, so a refused
//! extraction leaves no file and no directory. A successful extraction hands
//! the directory to a [`StagedBundle`], which removes it when it is dropped.
//!
//! **The archive is the untrusted side.** No entry's name is ever joined to the
//! staging root: a destination is built from the components an entry has to
//! match to be staged at all, and nothing else is written.

use std::ffi::OsStr;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Cursor, Read, Seek};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use zip::ZipArchive;

use super::VerifiedBundle;

/// The prefix every staging directory's name carries.
///
/// A staging directory is created under [`std::env::temp_dir()`] with this
/// prefix, this process's id and a counter, so a sweep after a crash can find
/// what a killed process left rather than guess at names.
pub const STAGING_PREFIX: &str = "metis-plugin-stage-";

/// The manifest's name at the archive root.
const MANIFEST_NAME: &str = "manifest.toml";

/// The directory each target's library lives in, under the archive root.
const LIBRARY_DIRECTORY: &str = "lib";

/// The mode a staged directory carries.
const STAGING_DIRECTORY_MODE: u32 = 0o700;

/// The mode a staged file carries. It is also the only mode an entry may
/// declare: what an archive says about permissions never reaches the
/// filesystem.
const STAGED_FILE_MODE: u32 = 0o600;

/// How many names are tried before the staging directory is called
/// unavailable.
const STAGING_ATTEMPTS: u32 = 16;

/// The staging names handed out by this process.
static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A staged bundle: the directory this host's load reads, removed when it is
/// dropped.
///
/// The caller holds this value for as long as the load needs the files, which
/// is what keeps a staged library alive until its instance is finished with it.
/// Dropping it removes the staging directory and everything in it.
#[derive(Debug)]
pub struct StagedBundle {
    root: PathBuf,
    manifest: PathBuf,
    library: PathBuf,
}

impl StagedBundle {
    /// The staging directory, which holds the staged files and nothing else.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the archive's root manifest is staged.
    ///
    /// Whether a manifest is there is the manifest step's question rather than
    /// extraction's: extraction answers where the file would be.
    pub fn manifest_path(&self) -> &Path {
        &self.manifest
    }

    /// Where this host's target library is staged.
    pub fn library_path(&self) -> &Path {
        &self.library
    }
}

impl Drop for StagedBundle {
    fn drop(&mut self) {
        // The staging directory is this value's own, and a removal that fails
        // is not worth panicking in a drop: the name is documented and a sweep
        // can find what is left.
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// What refusing an archive can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageError {
    /// The bytes are not a ZIP archive this build can read.
    ArchiveUnreadable { detail: String },
    /// The archive holds more entries than the limit allows.
    TooManyEntries { count: usize, limit: usize },
    /// An entry is larger than the per-entry limit.
    EntryTooLarge { name: String, size: u64, limit: u64 },
    /// The entries declare more uncompressed bytes between them than the limit
    /// allows.
    ArchiveTooLarge { declared_size: u64, limit: u64 },
    /// An entry's name is absolute, by this host's rules or by Windows's, so
    /// joining it to the staging root would replace it.
    AbsoluteEntryName { name: String },
    /// An entry's name carries a `..` component, so it would escape the staging
    /// root.
    ParentEntryName { name: String },
    /// An entry's name is not usable as a relative path: it is empty once its
    /// separators are removed, or it carries a NUL byte.
    UnusableEntryName { name: String },
    /// An entry's name is not text this build can read.
    UnreadableEntryName { index: usize, detail: String },
    /// An entry is a symbolic link.
    EntryIsSymlink { name: String },
    /// An entry is a directory. The archive's layout does not need one: the
    /// host creates every directory it writes into.
    EntryIsDirectory { name: String },
    /// An entry declares a file type that is not a regular file: a hard link,
    /// a device, a socket or a FIFO.
    EntryIsNotRegularFile { name: String, mode: u32 },
    /// An entry declares permission bits other than the ones this host writes.
    EntryModeRefused { name: String, mode: u32 },
    /// An entry's bytes did not read: a checksum that disagrees, a malformed
    /// stream, or a compression method this build does not read.
    EntryUnreadable { name: String, detail: String },
    /// The archive holds no library for this host's target.
    MissingTargetLibrary { target: String },
    /// The archive holds more than one file directly under this host's target
    /// directory, so which one is the library is not decided.
    AmbiguousTargetLibrary { target: String, files: usize },
    /// A filesystem operation on the staging directory failed.
    Io {
        operation: &'static str,
        path: PathBuf,
        detail: String,
    },
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StageError::ArchiveUnreadable { detail } => {
                write!(f, "the archive does not read: {detail}")
            }
            StageError::TooManyEntries { count, limit } => write!(
                f,
                "the archive holds {count} entries, and it may hold {limit}"
            ),
            StageError::EntryTooLarge { name, size, limit } => write!(
                f,
                "the entry {name} is {size} bytes, and an entry may be {limit}"
            ),
            StageError::ArchiveTooLarge {
                declared_size,
                limit,
            } => write!(
                f,
                "the entries declare {declared_size} bytes, and an archive may declare {limit}"
            ),
            StageError::AbsoluteEntryName { name } => write!(
                f,
                "the entry name {name} is absolute, and every name must be relative to the archive root"
            ),
            StageError::ParentEntryName { name } => write!(
                f,
                "the entry name {name} carries a parent directory, and no name may leave the archive root"
            ),
            StageError::UnusableEntryName { name } => {
                write!(f, "the entry name {name:?} is not a usable relative path")
            }
            StageError::UnreadableEntryName { index, detail } => write!(
                f,
                "the name of the entry at index {index} does not read: {detail}"
            ),
            StageError::EntryIsSymlink { name } => {
                write!(f, "the entry {name} is a symbolic link")
            }
            StageError::EntryIsDirectory { name } => {
                write!(f, "the entry {name} is a directory")
            }
            StageError::EntryIsNotRegularFile { name, mode } => write!(
                f,
                "the entry {name} declares the file type {mode:#o}, and only a regular file is read"
            ),
            StageError::EntryModeRefused { name, mode } => write!(
                f,
                "the entry {name} declares the mode {mode:#o}, and this host writes {STAGED_FILE_MODE:#o}"
            ),
            StageError::EntryUnreadable { name, detail } => {
                write!(f, "the entry {name} does not read: {detail}")
            }
            StageError::MissingTargetLibrary { target } => {
                write!(f, "the archive holds no library for the target {target}")
            }
            StageError::AmbiguousTargetLibrary { target, files } => write!(
                f,
                "the target directory {target} holds {files} files, and exactly one is the library"
            ),
            StageError::Io {
                operation,
                path,
                detail,
            } => write!(
                f,
                "the staging directory could not be written: {operation} at {} failed: {detail}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for StageError {}

/// Extracts a verified bundle's archive for `target`.
///
/// `target` is this host's target as the contract spells it, for example
/// `linux-x86_64`. The archive's root manifest and the files directly under
/// `lib/<target>/` are staged; every other entry is validated and skipped. The
/// returned [`StagedBundle`] owns the staging directory.
pub fn extract(verified: &VerifiedBundle<'_>, target: &str) -> Result<StagedBundle, StageError> {
    let root = create_staging_directory()?;
    match stage(verified, target, &root) {
        Ok(staged) => Ok(staged),
        Err(error) => {
            // A refused extraction leaves nothing staged, whatever it wrote
            // before the refusal.
            let _ = fs::remove_dir_all(&root);
            Err(error)
        }
    }
}

/// Stages the archive under `root`, which the caller owns and removes.
fn stage(
    verified: &VerifiedBundle<'_>,
    target: &str,
    root: &Path,
) -> Result<StagedBundle, StageError> {
    let bytes: &[u8] = verified.archive();
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|error| StageError::ArchiveUnreadable {
            detail: error.to_string(),
        })?;

    let mut library: Option<PathBuf> = None;

    for index in 0..archive.len() {
        // The metadata pass reads no entry's bytes, so an archive is refused on
        // its names and its declared metadata before anything is decompressed.
        let (name, entry_path) = {
            let entry =
                archive
                    .by_index_data(index)
                    .map_err(|error| StageError::ArchiveUnreadable {
                        detail: error.to_string(),
                    })?;
            let name = entry
                .name()
                .map_err(|error| StageError::UnreadableEntryName {
                    index,
                    detail: error.to_string(),
                })?
                .into_owned();
            let entry_path = PathBuf::from(&name);
            (name, entry_path)
        };

        let kind = classify(&entry_path, target);
        let destination = match &kind {
            Entry::Manifest => root.join(MANIFEST_NAME),
            Entry::TargetLibrary(file_name) => {
                root.join(LIBRARY_DIRECTORY).join(target).join(file_name)
            }
            Entry::Other => continue,
        };
        write_entry(&mut archive, index, &destination, &name)?;
        if let Entry::TargetLibrary(_) = kind {
            library = Some(destination);
        }
    }

    let Some(library) = library else {
        return Err(StageError::MissingTargetLibrary {
            target: target.to_string(),
        });
    };
    Ok(StagedBundle {
        root: root.to_path_buf(),
        manifest: root.join(MANIFEST_NAME),
        library,
    })
}

/// What one entry is, under the archive root.
enum Entry {
    /// The manifest at the archive root.
    Manifest,
    /// A file directly under this host's target directory.
    TargetLibrary(String),
    /// Anything else: not staged.
    Other,
}

/// Classifies an archive-relative path.
///
/// A path reaches a destination only by matching one of the archive's two
/// shapes exactly, so an entry that names a parent directory, an absolute path
/// or a deeper tree matches nothing and is not written.
fn classify(relative: &Path, target: &str) -> Entry {
    let names: Vec<&OsStr> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    match names.as_slice() {
        [name] if *name == OsStr::new(MANIFEST_NAME) => Entry::Manifest,
        [directory, entry_target, file_name]
            if *directory == OsStr::new(LIBRARY_DIRECTORY)
                && *entry_target == OsStr::new(target) =>
        {
            Entry::TargetLibrary(file_name.to_string_lossy().into_owned())
        }
        _ => Entry::Other,
    }
}

/// Writes one entry's bytes to `path`.
///
/// The reader bounds what it yields by the entry's declared size, so the bytes
/// written to the staging directory are bounded by that declaration.
fn write_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    index: usize,
    path: &Path,
    name: &str,
) -> Result<(), StageError> {
    if let Some(parent) = path.parent() {
        create_directory(parent)?;
    }
    let mut entry = archive
        .by_index(index)
        .map_err(|error| StageError::EntryUnreadable {
            name: name.to_string(),
            detail: error.to_string(),
        })?;
    let mut file = create_file(path)?;
    io::copy(&mut entry, &mut file).map_err(|error| StageError::EntryUnreadable {
        name: name.to_string(),
        detail: error.to_string(),
    })?;
    Ok(())
}

/// Creates a staging directory, optionally with its parents.
fn create_directory(path: &Path) -> Result<(), StageError> {
    create_directory_at(path).map_err(|error| StageError::Io {
        operation: "create a staged directory",
        path: path.to_path_buf(),
        detail: error.to_string(),
    })
}

/// Creates a directory with this host's own mode.
fn create_directory_at(path: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(STAGING_DIRECTORY_MODE);
    }
    builder.create(path)
}

/// Creates one staged file with this host's own mode.
fn create_file(path: &Path) -> Result<File, StageError> {
    let file = open_staged_file(path).map_err(|error| StageError::Io {
        operation: "create a staged file",
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    // The mode the host writes, whatever the process's umask says.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(STAGED_FILE_MODE))
            .map_err(|error| StageError::Io {
                operation: "set a staged file's mode",
                path: path.to_path_buf(),
                detail: error.to_string(),
            })?;
    }
    Ok(file)
}

/// Opens a new staged file, never reusing an existing name.
fn open_staged_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(STAGED_FILE_MODE);
    }
    options.open(path)
}

/// Creates a fresh staging directory and returns its path.
fn create_staging_directory() -> Result<PathBuf, StageError> {
    let base = std::env::temp_dir();
    let process = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0);
    for _ in 0..STAGING_ATTEMPTS {
        let counter = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!("{STAGING_PREFIX}{process}-{nanos}-{counter}"));
        match create_directory_at(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(StageError::Io {
                    operation: "create the staging directory",
                    path,
                    detail: error.to_string(),
                });
            }
        }
    }
    Err(StageError::Io {
        operation: "create the staging directory",
        path: base,
        detail: format!("no name among {STAGING_ATTEMPTS} attempts was free"),
    })
}
