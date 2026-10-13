//! The daemon's state store, exercised through its public interface.
//!
//! The store operations run against `sqlite::memory:`; the creation and
//! migration seams run against a temporary file, because those are properties
//! of the file the daemon keeps rather than of a query.

use std::path::PathBuf;

use daemon::state::Store;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;

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

/// An independent connection to the file the store created, opened the way an
/// operator's sqlite client would open it. It is never the store's own pool, so
/// what it reads is what is on disk.
async fn open_raw(path: &std::path::Path) -> SqlitePool {
    SqlitePool::connect_with(SqliteConnectOptions::new().filename(path))
        .await
        .expect("the database the store created opens")
}

/// A fresh file is usable without a separate migration step: opening it applies
/// the committed migration, so the token table is there and the migration is
/// recorded as applied. Opening the same file again changes nothing.
#[tokio::test]
async fn a_fresh_database_is_migrated_on_open() {
    let base = TempDir::new();
    let database = base.join("state/metis.db");

    let store = Store::open(&database).await.expect("a fresh path opens");

    let raw = open_raw(&database).await;
    let table: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'tokens'",
    )
    .fetch_one(&raw)
    .await
    .expect("the schema is readable");
    assert_eq!(table, 1, "the migration creates the token table on open");
    let applied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE version = 1 AND success = 1",
    )
    .fetch_one(&raw)
    .await
    .expect("the migration ledger is readable");
    assert_eq!(applied, 1, "the migration is recorded as applied");
    raw.close().await;
    drop(store);

    // A migration that was not recorded would be applied a second time, and the
    // `CREATE TABLE` would fail on the table that is already there. Opening the
    // same file twice is therefore the whole of idempotence.
    let reopened = Store::open(&database)
        .await
        .expect("opening the same file again applies nothing");
    drop(reopened);
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
