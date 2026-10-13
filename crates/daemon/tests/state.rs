//! The daemon's state store, exercised through its public interface.
//!
//! The store operations run against `sqlite::memory:`; the creation and
//! migration seams run against a temporary file, because those are properties
//! of the file the daemon keeps rather than of a query.

use std::path::PathBuf;

use daemon::state::Store;

/// A temporary directory that removes itself on drop, so the test cleans up on
/// the success path and on a panic alike.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "metis-state-store-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the temporary directory");
        Self { path }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The mode a path carries, read the way an operator reads it.
#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::MetadataExt;

    std::fs::symlink_metadata(path)
        .expect("the path exists")
        .mode()
        & 0o7777
}

/// The daemon owns the directory it keeps its state in: the directory is
/// created private to its own user and the database file owner-only, because
/// the file carries the token hashes.
#[cfg(unix)]
#[tokio::test]
async fn a_fresh_database_is_created_private() {
    let base = TempDir::new();
    let database = base.join("state/metis.db");
    let directory = database.parent().expect("the database has a directory");

    let store = Store::open(&database)
        .await
        .expect("a fresh path opens without preparation");

    assert_eq!(
        mode_of(directory),
        0o700,
        "the state directory is private to the daemon's user"
    );
    assert_eq!(mode_of(&database), 0o600, "the database file is owner-only");
    drop(store);
}
