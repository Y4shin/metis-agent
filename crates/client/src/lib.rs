//! The UI-independent client for the daemon.
//!
//! The wrapper hides the local transport, so a caller constructs a client and
//! calls a method without naming a transport, an `Http2Connection`, or a
//! generated stub type. `skeleton` holds the test-support service the
//! walking-skeleton test and example drive; the daemon effort replaces it.

pub mod skeleton;
