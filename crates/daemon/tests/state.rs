//! The daemon's state store, exercised through its public interface.
//!
//! The store operations run against `sqlite::memory:`; the creation and
//! migration seams run against a temporary file, because those are properties
//! of the file the daemon keeps rather than of a query.

use std::path::PathBuf;

use daemon::state::{StateError, Store, Verify};
use daemon::token::mint;
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

/// An independent connection to the file the store created, opened the way an
/// operator's sqlite client would open it. It is never the store's own pool, so
/// what it reads is what is on disk.
async fn open_raw(path: &std::path::Path) -> SqlitePool {
    SqlitePool::connect_with(SqliteConnectOptions::new().filename(path))
        .await
        .expect("the database the store created opens")
}

/// Every byte the daemon keeps for this database: the file and, when the
/// journal is beside it, the journal too.
fn database_bytes(path: &std::path::Path) -> Vec<u8> {
    let mut bytes = std::fs::read(path).expect("the database file is readable");
    for suffix in ["-wal", "-journal"] {
        let mut sidecar = path.as_os_str().to_owned();
        sidecar.push(suffix);
        if let Ok(extra) = std::fs::read(PathBuf::from(sidecar)) {
            bytes.extend(extra);
        }
    }
    bytes
}

/// Whether `haystack` holds `needle` at any offset.
fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// A token row round-trips: the store keeps the hash and verifies it, and the
/// row never holds the token itself. Verification reports an outcome and never
/// hands the stored hash back to its caller.
#[tokio::test]
async fn a_token_row_stores_the_hash_and_verifies_it() {
    let base = TempDir::new();
    let database = base.join("state/metis.db");
    let store = Store::open(&database).await.expect("a fresh path opens");
    let (token, hash) = mint();

    store
        .insert_token("desktop", &hash)
        .await
        .expect("the surface pairs");
    assert_eq!(
        store
            .verify_token("desktop", &hash)
            .await
            .expect("the database answers"),
        Verify::Verified,
        "the token the surface was given verifies"
    );

    // Another token of the same shape, and a surface that was never paired:
    // the answer is the outcome, not the stored row.
    let (_, other) = mint();
    assert_eq!(
        store
            .verify_token("desktop", &other)
            .await
            .expect("the database answers"),
        Verify::Mismatch,
        "a different token is refused"
    );
    assert_eq!(
        store
            .verify_token("laptop", &hash)
            .await
            .expect("the database answers"),
        Verify::UnknownSurface,
        "an unpaired surface has no row"
    );
    drop(store);

    // The row itself, read with an independent connection: the surface name and
    // the 32-byte digest, and nothing else.
    let raw = open_raw(&database).await;
    let (surface, stored): (String, Vec<u8>) = sqlx::query_as("SELECT surface, hash FROM tokens")
        .fetch_one(&raw)
        .await
        .expect("the row is readable");
    assert_eq!(surface, "desktop", "the row names the surface");
    assert_eq!(
        stored,
        hash.as_bytes().to_vec(),
        "the row holds the SHA-256 of the token"
    );
    raw.close().await;

    // And nowhere in what the daemon keeps is the token itself.
    assert!(
        !holds(&database_bytes(&database), token.as_str().as_bytes()),
        "the token text is not in the file the daemon keeps"
    );
}

/// Revoking one surface leaves the other surfaces verifying: a token belongs to
/// one surface, and revoking one does not disturb the rest.
#[tokio::test]
async fn revoking_one_surface_leaves_the_other_verifying() {
    let store = Store::open_in_memory()
        .await
        .expect("the store opens in memory");
    let (_, desktop_hash) = mint();
    let (_, phone_hash) = mint();
    store
        .insert_token("desktop", &desktop_hash)
        .await
        .expect("one surface pairs");
    store
        .insert_token("phone", &phone_hash)
        .await
        .expect("another surface pairs");
    assert_eq!(
        store.surfaces().await.expect("the surfaces are listed"),
        vec!["desktop".to_owned(), "phone".to_owned()]
    );

    store
        .revoke_token("desktop")
        .await
        .expect("the row is removed");

    assert_eq!(
        store
            .verify_token("desktop", &desktop_hash)
            .await
            .expect("the database answers"),
        Verify::UnknownSurface,
        "the revoked surface no longer verifies"
    );
    assert_eq!(
        store
            .verify_token("phone", &phone_hash)
            .await
            .expect("the database answers"),
        Verify::Verified,
        "the other surface still verifies"
    );
    assert_eq!(
        store.surfaces().await.expect("the surfaces are listed"),
        vec!["phone".to_owned()]
    );

    // Revoking a surface that has no row is not a failure: the caller asked for
    // the state it gets.
    store
        .revoke_token("desktop")
        .await
        .expect("revoking twice is not an error");
}

/// Pairing a surface that already has a row is refused with a typed error, and
/// the row that is already there stays exactly as it was: a second pairing never
/// replaces the first surface's token.
#[tokio::test]
async fn an_insert_for_a_paired_surface_is_refused() {
    let store = Store::open_in_memory()
        .await
        .expect("the store opens in memory");
    let (_, first) = mint();
    let (_, second) = mint();
    store
        .insert_token("desktop", &first)
        .await
        .expect("the surface pairs once");

    let error = store
        .insert_token("desktop", &second)
        .await
        .expect_err("the second pairing is refused");
    assert!(
        matches!(error, StateError::SurfaceAlreadyPaired),
        "the refusal is typed rather than a database error: {error:?}"
    );

    assert_eq!(
        store
            .verify_token("desktop", &first)
            .await
            .expect("the database answers"),
        Verify::Verified,
        "the first token still verifies"
    );
    assert_eq!(
        store
            .verify_token("desktop", &second)
            .await
            .expect("the database answers"),
        Verify::Mismatch,
        "the refused token was not stored"
    );
    assert_eq!(
        store.surfaces().await.expect("the surfaces are listed"),
        vec!["desktop".to_owned()]
    );
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
