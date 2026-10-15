//! One boundary for what a plugin says outside its response payload.
//!
//! A plugin reports progress, emits an event, writes a log line and asks for a
//! service through the host table's four callbacks. This module is the host's
//! one destination for all four: a daemon implements [`Sink`] and installs it
//! with [`Host::with_sink`](crate::Host::with_sink), and the runtime delivers
//! every callback there, naming the instance that spoke.
//!
//! Three rules shape the surface, and they follow from where a callback is
//! delivered rather than from taste:
//!
//! **A callback happens inside the plugin's poll frame.** The sink is called
//! from the `extern "C"` frame the plugin invoked, so a sink method cannot
//! fail, must not block and must not call back into the runtime or the plugin.
//! A slow destination stalls one plugin's poll and nothing else, because the
//! runtime keeps no queue on a sink's behalf: a daemon that wants
//! backpressure puts its own queue behind its sink.
//!
//! **A payload is a copy the host owns.** The plugin's buffer is copied and
//! released through the plugin's own release entry before the sink is called,
//! so a sink never holds a reference into plugin memory and never releases a
//! buffer itself.
//!
//! **A panic is caught in the callback's own frame.** A panic inside a sink is
//! recorded on the ordering ledger and the plugin's call continues, because
//! only the callback's own frame can catch what the plugin's poll frame would
//! otherwise carry.

use std::sync::{Arc, Mutex};

use plugin_contract::abi::MetisStatus;

use crate::PendingService;

/// The four levels the frozen `log` callback carries as the integers 0 to 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Level 0.
    Debug,
    /// Level 1.
    Info,
    /// Level 2.
    Warn,
    /// Level 3.
    Error,
}

impl LogLevel {
    /// The level an ABI integer names.
    ///
    /// `None` is a value outside the four the contract documents, which is a
    /// protocol violation rather than a fifth level: this build never invents
    /// one.
    pub const fn from_abi(level: i32) -> Option<LogLevel> {
        match level {
            0 => Some(LogLevel::Debug),
            1 => Some(LogLevel::Info),
            2 => Some(LogLevel::Warn),
            3 => Some(LogLevel::Error),
            _ => None,
        }
    }

    /// The integer the frozen callback carries for this level.
    pub const fn to_abi(self) -> i32 {
        match self {
            LogLevel::Debug => 0,
            LogLevel::Info => 1,
            LogLevel::Warn => 2,
            LogLevel::Error => 3,
        }
    }

    /// The level's wire name, for a ledger line or a log line.
    pub const fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// What a host delivers to the sink a daemon installs.
///
/// Every method is infallible and must not block, because it runs inside the
/// plugin's poll frame: a destination that can refuse a record or wait for room
/// keeps that policy behind its own queue. The instance name is the name the
/// instance was loaded under, and every method carries it, so one host serving
/// several instances can tell which plugin spoke. Progress and a service
/// request carry the request id they belong to, an event carries its topic and
/// its payload and no request id, because the frozen callback has none, and a
/// log carries its typed level.
///
/// A service request's return value is the status the frozen callback expects:
/// the sink accepts or refuses synchronously, and an accepted request is
/// answered later through [`Instance::complete_service`](crate::Instance::complete_service).
pub trait Sink: Send + Sync + 'static {
    /// One intermediate output while a call is still running.
    fn progress(&self, instance: &str, request_id: u64, payload: Vec<u8>);

    /// One fire-and-forget notification, which no call's completion depends on.
    fn event(&self, instance: &str, topic: &str, payload: Vec<u8>);

    /// One log line at the level the frozen callback carried.
    fn log(&self, instance: &str, level: LogLevel, message: &str);

    /// The plugin asks for a service. The answer is delivered later.
    fn service_request(
        &self,
        instance: &str,
        service: &str,
        request_id: u64,
        payload: Vec<u8>,
    ) -> MetisStatus;
}

/// What the default sink records, and what the host's accessors read.
///
/// It is the storage the recording sink and [`Host`](crate::Host)'s accessors
/// share, so a host with no sink of its own keeps answering `logs()`,
/// `events()`, `pending_services()` and `take_service()` exactly as it did
/// before a sink existed. A host with a sink of its own leaves it empty.
#[derive(Default)]
pub(crate) struct Records {
    pub(crate) logs: Mutex<Vec<String>>,
    pub(crate) events: Mutex<Vec<String>>,
    pub(crate) services: Mutex<Vec<PendingService>>,
}

/// The sink a host has when the caller installed none.
///
/// It is what keeps a host with no sink legal: the same records the accessors
/// have always read, the same progress line the host has always printed, and
/// the same one service (`timer`) the host has always accepted. It is not
/// public, because a caller that wants it wants the accessors, and a caller
/// that wants a destination implements [`Sink`] itself.
pub(crate) struct RecordingSink {
    records: Arc<Records>,
}

impl RecordingSink {
    pub(crate) fn new(records: Arc<Records>) -> RecordingSink {
        RecordingSink { records }
    }
}

impl Sink for RecordingSink {
    fn progress(&self, _instance: &str, request_id: u64, payload: Vec<u8>) {
        // The progress payload has no accessor, and the host has always printed
        // this line. The default sink is where that belongs now that the
        // delivery is one object's business.
        println!(
            "[runtime] progress for request {request_id}: {} bytes",
            payload.len()
        );
    }

    fn event(&self, _instance: &str, topic: &str, _payload: Vec<u8>) {
        crate::lock(&self.records.events).push(topic.to_string());
    }

    fn log(&self, _instance: &str, level: LogLevel, message: &str) {
        crate::lock(&self.records.logs).push(format!("[{}] {message}", level.to_abi()));
    }

    fn service_request(
        &self,
        _instance: &str,
        service: &str,
        request_id: u64,
        payload: Vec<u8>,
    ) -> MetisStatus {
        if service != "timer" {
            return MetisStatus::NotFound;
        }
        crate::lock(&self.records.services).push(PendingService {
            service: service.to_string(),
            request_id,
            body: payload,
        });
        MetisStatus::Ok
    }
}
