//! PROTOTYPE (throwaway). Not product code.
//!
//! The ABI adapter: a cdylib loaded with `libloading`, behind the same `Plugin`
//! seam as the native in-process plugin.
//!
//! Round two does not reimplement the pending registry. It drives the cdylib
//! with the **landed** `plugin_contract::authoring::HostAdapter`, which already
//! owns the slot, the waker, the bounded wait, the dropped-completion path, and
//! the plugin-buffer release. The adapter here is the thin wrapper that turns
//! that into the seam: a per-call `HostAdapter::register` before
//! `HostAdapter::invoke`, and a future that awaits the `Await` the register
//! returned.
//!
//! The one place the landed adapter does not fit: `HostAdapter::register`
//! followed by a non-`Pending`, non-`Ok` `invoke` (a synchronous plugin error)
//! leaves the slot registered, because the adapter exposes no way to retire a
//! slot except through a successful `cancel`. This prototype avoids the leak by
//! only ever calling `invoke` for capabilities the registry has already
//! admitted; a real host needs `invoke` to retire its own slot on a synchronous
//! error, or an explicit `unregister`.

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use plugin_contract::abi::{
    METIS_ABI_MAJOR, MetisBuffer, MetisPluginEntryV1Fn, MetisPluginTableV1, MetisStatus,
    metis_read_prefix,
};
use plugin_contract::authoring::{Await, HostAdapter};
use serde::Deserialize;
use serde_json::Value;

use proto_registry::{
    BoxFuture, Call, CapabilityDescriptor, Payload, Plugin, PluginError, PluginManifest, Status,
};

/// The order in which the host shut the plugin down and unloaded the library,
/// so the teardown ordering is proven with a marker and not with prose.
pub type OrderLedger = Arc<Mutex<Vec<String>>>;

/// The `libloading::Library`, dropped only after `AbiPlugin`'s `Drop` has run
/// `shutdown`. It is declared last in `AbiPlugin` so its field drop comes last.
struct TrackedLibrary {
    library: libloading::Library,
    ledger: OrderLedger,
}

impl Drop for TrackedLibrary {
    fn drop(&mut self) {
        self.ledger
            .lock()
            .unwrap()
            .push("library unloaded".to_owned());
        let _ = &self.library;
    }
}

/// A loaded cdylib behind the seam.
pub struct AbiPlugin {
    adapter: HostAdapter,
    instance: *mut c_void,
    table: *const MetisPluginTableV1,
    manifest: PluginManifest,
    shut_down: AtomicBool,
    capability_list_bytes: AtomicUsize,
    schema_fetch_calls: AtomicUsize,
    schema_fetch_bytes: AtomicUsize,
    inline_bytes: AtomicUsize,
    ledger: OrderLedger,
    /// Declared last: drops last, after `Drop::drop` has shut the instance down.
    library: TrackedLibrary,
}

// SAFETY: the library stays mapped for the plugin's lifetime, the table is
// immutable, the instance is only reached through `&self` methods, and the C
// side is required to be thread safe. Nothing in the ABI enforces the last
// part, which is one of round one's findings.
unsafe impl Send for AbiPlugin {}
unsafe impl Sync for AbiPlugin {}

impl AbiPlugin {
    /// Loads the library, reads the version 1 table through `metis_read_prefix`,
    /// and creates one instance.
    pub fn load(path: &Path, id: &str, ledger: OrderLedger) -> Result<AbiPlugin, String> {
        // SAFETY: loading a library runs its initialisers; the caller vets the
        // path, which for this prototype is the cdylib next to the host binary.
        let library = unsafe { libloading::Library::new(path) }
            .map_err(|err| format!("could not load {}: {err}", path.display()))?;

        let table = {
            // SAFETY: the symbol name is the frozen v1 entry point.
            let entry: libloading::Symbol<MetisPluginEntryV1Fn> =
                unsafe { library.get(plugin_contract::abi::METIS_PLUGIN_ENTRY_V1) }
                    .map_err(|err| format!("no metis_plugin_entry_v1 symbol: {err}"))?;
            // SAFETY: the entry point returns a live table.
            unsafe { entry() }
        };

        // SAFETY: the table comes from the entry point we just called.
        let prefix = unsafe { metis_read_prefix(table) }.ok_or_else(|| {
            "the plugin table does not cover the size and version prefix".to_owned()
        })?;
        if prefix.abi_major != METIS_ABI_MAJOR {
            return Err(format!(
                "the plugin speaks ABI major {}, this host speaks {METIS_ABI_MAJOR}",
                prefix.abi_major
            ));
        }

        let adapter = HostAdapter::new();
        // The adapter releases every plugin buffer through the plugin's own
        // `release_buffer`, which requires knowing the table.
        adapter.attach(table);

        let mut instance: *mut c_void = std::ptr::null_mut();
        // SAFETY: the table is live, and the host table outlives the instance.
        let status = unsafe { ((*table).create_instance)(adapter.api(), &mut instance) };
        if status != MetisStatus::Ok {
            return Err(format!("create_instance returned {}", status.as_str()));
        }

        Ok(AbiPlugin {
            adapter,
            instance,
            table,
            manifest: PluginManifest {
                id: id.to_owned(),
                version: format!("abi {}.{}", prefix.abi_major, prefix.abi_minor),
                abi_major: prefix.abi_major,
            },
            shut_down: AtomicBool::new(false),
            capability_list_bytes: AtomicUsize::new(0),
            schema_fetch_calls: AtomicUsize::new(0),
            schema_fetch_bytes: AtomicUsize::new(0),
            inline_bytes: AtomicUsize::new(0),
            library: TrackedLibrary {
                library,
                ledger: Arc::clone(&ledger),
            },
            ledger,
        })
    }

    /// The log lines the plugin sent, with their levels.
    pub fn host_logs(&self) -> Vec<(i32, String)> {
        self.adapter.logs()
    }

    pub fn saw_log(&self, needle: &str) -> bool {
        self.host_logs()
            .iter()
            .any(|(_, line)| line.contains(needle))
    }

    /// The events the plugin sent: topic and envelope-stripped body.
    pub fn events(&self) -> Vec<(String, Vec<u8>)> {
        self.adapter.events()
    }

    /// How many callback bodies panicked and were caught at their own frame.
    pub fn callback_panics(&self) -> usize {
        self.adapter.callback_panics()
    }

    pub fn capability_list_bytes(&self) -> usize {
        self.capability_list_bytes.load(SeqCst)
    }

    pub fn schema_fetch_calls(&self) -> usize {
        self.schema_fetch_calls.load(SeqCst)
    }

    pub fn schema_fetch_bytes(&self) -> usize {
        self.schema_fetch_bytes.load(SeqCst)
    }

    /// The size in bytes of the inline-schema descriptor listing, measured
    /// through the ABI (the round one shape), for the descriptor size trade-off.
    pub fn measure_inline_descriptor_bytes(&self) -> Result<usize, String> {
        let body = self
            .call_pending("metis.descriptors_inline", b"{}", Duration::from_secs(5))
            .map_err(|err| err.to_string())?;
        self.inline_bytes.store(body.len(), SeqCst);
        Ok(body.len())
    }

    pub fn inline_descriptor_bytes(&self) -> usize {
        self.inline_bytes.load(SeqCst)
    }

    /// Registers a pending request, calls `invoke`, and waits for the completion
    /// with a bound. Used for the host-internal `metis.*` calls, which are
    /// synchronous seam calls (`schemas`, `configure`) off the async path.
    pub fn call_pending(
        &self,
        capability: &str,
        body: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, PluginError> {
        let request_id = Call::new().id();
        let awaiting = self.adapter.register(request_id);
        // SAFETY: the instance is live for the plugin's lifetime.
        let status = unsafe {
            self.adapter
                .invoke(self.instance, capability, request_id, body)
        };
        if status != MetisStatus::Pending {
            return Err(map_status(status, capability));
        }
        match awaiting.wait_timeout(timeout) {
            Some(completed) if completed.status == MetisStatus::Ok => Ok(completed.body),
            Some(completed) => Err(map_status(completed.status, capability)),
            None => Err(PluginError::Internal {
                detail: format!("no completion for {capability} within {timeout:?}"),
            }),
        }
    }

    /// The bounded wait, for the completion that never arrives.
    pub fn invoke_bounded(
        &self,
        capability: &str,
        payload: &Payload,
        timeout: Duration,
    ) -> Result<Payload, PluginError> {
        let request_id = Call::new().id();
        let awaiting = self.adapter.register(request_id);
        // SAFETY: the instance is live for the plugin's lifetime.
        let status = unsafe {
            self.adapter
                .invoke(self.instance, capability, request_id, payload.as_bytes())
        };
        if status != MetisStatus::Pending {
            return Err(map_status(status, capability));
        }
        match awaiting.wait_timeout(timeout) {
            Some(completed) if completed.status == MetisStatus::Ok => {
                Ok(Payload::from_bytes(completed.body))
            }
            Some(completed) => Err(map_status(completed.status, capability)),
            None => Err(PluginError::Internal {
                detail: format!("no completion for {capability} within {timeout:?}"),
            }),
        }
    }

    /// Cancels a pending request directly through the landed adapter, which
    /// retires the host's slot on a successful plugin cancel.
    pub fn cancel_raw(&self, request_id: u64) -> Result<(), PluginError> {
        // SAFETY: the instance is live for the plugin's lifetime.
        let status = unsafe { self.adapter.cancel(self.instance, request_id) };
        if status == MetisStatus::Ok {
            Ok(())
        } else {
            Err(map_status(status, "cancel"))
        }
    }

    /// The raw instance handle, for the instance lifecycle checks.
    pub fn instance(&self) -> *mut c_void {
        self.instance
    }

    /// Binds a host-polled future driver to this plugin's table and instance.
    /// The table pointer is reinterpreted as the appended layout: the landed
    /// prefix is byte for byte the same, so the offsets the driver reads are
    /// the ones the plugin wrote.
    pub fn future_host(&self, ledger: OrderLedger) -> crate::future_driver::FutureHost {
        crate::future_driver::FutureHost::new(
            self.table as *const proto_abi::MetisPluginTableV1,
            self.instance,
            ledger,
        )
    }

    /// Creates a further instance of this plugin through the same table.
    pub fn create_instance_handle(&self) -> Result<*mut c_void, String> {
        let mut instance: *mut c_void = std::ptr::null_mut();
        // SAFETY: the table is live and the out parameter is this frame's.
        let status =
            unsafe { ((*self.table).create_instance)(self.adapter.api(), &mut instance) };
        if status == MetisStatus::Ok {
            Ok(instance)
        } else {
            Err(format!("create_instance returned {}", status.as_str()))
        }
    }

    /// Destroys an instance handle through the table, reporting its status.
    pub fn destroy_instance_handle(&self, instance: *mut c_void) -> MetisStatus {
        // SAFETY: the caller supplies a handle from this table.
        unsafe { ((*self.table).destroy_instance)(instance) }
    }

    /// Registers, calls `invoke`, and reports whether the call returned
    /// `Pending` (never inline) together with the awaited result.
    pub fn probe_pending(
        &self,
        capability: &str,
        payload: &Payload,
    ) -> (bool, Result<Payload, PluginError>) {
        let request_id = Call::new().id();
        let awaiting = self.adapter.register(request_id);
        // SAFETY: the instance is live.
        let status = unsafe {
            self.adapter
                .invoke(self.instance, capability, request_id, payload.as_bytes())
        };
        if status != MetisStatus::Pending {
            return (false, Err(map_status(status, capability)));
        }
        match awaiting.wait_timeout(Duration::from_secs(5)) {
            Some(completed) if completed.status == MetisStatus::Ok => {
                (true, Ok(Payload::from_bytes(completed.body)))
            }
            Some(completed) => (true, Err(map_status(completed.status, capability))),
            None => (
                true,
                Err(PluginError::Internal {
                    detail: format!("no completion for {capability} within the bound"),
                }),
            ),
        }
    }

    /// Registers and invokes, handing back the request id, the awaitable
    /// completion, and the status, so a caller can cancel and observe the race
    /// without a thread parked on the future.
    pub fn begin(
        &self,
        capability: &str,
        payload: &Payload,
        instance: *mut c_void,
    ) -> Result<(u64, Await), PluginError> {
        let request_id = Call::new().id();
        let awaiting = self.adapter.register(request_id);
        // SAFETY: the caller supplies a live handle from this table.
        let status = unsafe {
            self.adapter
                .invoke(instance, capability, request_id, payload.as_bytes())
        };
        if status != MetisStatus::Pending {
            return Err(map_status(status, capability));
        }
        Ok((request_id, awaiting))
    }

    /// Fires a call and never consumes its completion: register, invoke, drop.
    pub fn fire(&self, capability: &str, payload: &Payload) -> Result<(), PluginError> {
        let request_id = Call::new().id();
        let _awaiting = self.adapter.register(request_id);
        // SAFETY: the instance is live.
        let status = unsafe {
            self.adapter
                .invoke(self.instance, capability, request_id, payload.as_bytes())
        };
        if status == MetisStatus::Pending {
            Ok(())
        } else {
            Err(map_status(status, capability))
        }
    }

    /// Invokes a capability on an explicitly supplied instance.
    pub fn invoke_on(
        &self,
        instance: *mut c_void,
        capability: &str,
        payload: &Payload,
        timeout: Duration,
    ) -> Result<Payload, PluginError> {
        let request_id = Call::new().id();
        let awaiting = self.adapter.register(request_id);
        // SAFETY: the caller supplies a live handle from this table.
        let status = unsafe {
            self.adapter
                .invoke(instance, capability, request_id, payload.as_bytes())
        };
        if status != MetisStatus::Pending {
            return Err(map_status(status, capability));
        }
        match awaiting.wait_timeout(timeout) {
            Some(completed) if completed.status == MetisStatus::Ok => {
                Ok(Payload::from_bytes(completed.body))
            }
            Some(completed) => Err(map_status(completed.status, capability)),
            None => Err(PluginError::Internal {
                detail: format!("no completion for {capability} within {timeout:?}"),
            }),
        }
    }
}

impl Plugin for AbiPlugin {
    fn manifest(&self) -> PluginManifest {
        self.manifest.clone()
    }

    fn capabilities(&self) -> Result<Vec<CapabilityDescriptor>, PluginError> {
        #[derive(Deserialize)]
        struct WireList {
            capabilities: Vec<CapabilityDescriptor>,
        }

        let mut out = MetisBuffer::empty();
        // SAFETY: the instance is live and the out buffer is this frame's.
        let status = unsafe { ((*self.table).list_capabilities)(self.instance, &mut out) };
        if status != MetisStatus::Ok {
            return Err(PluginError::Discovery {
                plugin: self.manifest.id.clone(),
                detail: format!("list_capabilities returned {}", status.as_str()),
            });
        }
        // `read_and_release` copies the body out and releases the plugin buffer
        // through the plugin's own `release_buffer`.
        let body = self
            .adapter
            .read_and_release(out)
            .map_err(|detail| PluginError::Discovery {
                plugin: self.manifest.id.clone(),
                detail,
            })?;
        self.capability_list_bytes.store(body.len(), SeqCst);
        let list: WireList =
            serde_json::from_slice(&body).map_err(|err| PluginError::Discovery {
                plugin: self.manifest.id.clone(),
                detail: format!("the capability list did not parse: {err}"),
            })?;
        Ok(list.capabilities)
    }

    fn schemas(&self, ids: &[String]) -> Result<BTreeMap<String, Vec<u8>>, PluginError> {
        #[derive(Deserialize)]
        struct WireSchemas {
            schemas: BTreeMap<String, Value>,
        }

        let request = serde_json::to_vec(&serde_json::json!({ "ids": ids })).unwrap_or_default();
        self.schema_fetch_calls.fetch_add(1, SeqCst);
        let body = self.call_pending("metis.schemas", &request, Duration::from_secs(5))?;
        self.schema_fetch_bytes.fetch_add(body.len(), SeqCst);
        let parsed: WireSchemas =
            serde_json::from_slice(&body).map_err(|err| PluginError::SchemaMissing {
                plugin: self.manifest.id.clone(),
                schema_id: "<batch>".to_owned(),
                detail: format!("the schema response did not parse: {err}"),
            })?;
        Ok(parsed
            .schemas
            .into_iter()
            .map(|(id, value)| (id, serde_json::to_vec(&value).unwrap_or_default()))
            .collect())
    }

    fn configure(&self, configuration: &[u8]) -> Result<(), PluginError> {
        self.call_pending("metis.configure", configuration, Duration::from_secs(5))
            .map(|_| ())
    }

    fn invoke<'a>(
        &'a self,
        capability: &'a str,
        payload: Payload,
        call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>> {
        // D11: register before `invoke`, so a completion that overtakes the
        // return still finds its request. The request id is the caller's, so the
        // caller can cancel it.
        let request_id = call.id();
        let awaiting = self.adapter.register(request_id);
        // SAFETY: the instance is live for the plugin's lifetime.
        let status = unsafe {
            self.adapter
                .invoke(self.instance, capability, request_id, payload.as_bytes())
        };
        Box::pin(async move {
            if status != MetisStatus::Pending {
                return Err(map_status(status, capability));
            }
            let completed = awaiting.await;
            if completed.status == MetisStatus::Ok {
                Ok(Payload::from_bytes(completed.body))
            } else {
                Err(map_status(completed.status, capability))
            }
        })
    }

    fn cancel(&self, request_id: u64) -> Result<(), PluginError> {
        self.cancel_raw(request_id)
    }

    fn shutdown(&self) -> Result<(), PluginError> {
        if self.shut_down.swap(true, SeqCst) {
            return Ok(());
        }
        // SAFETY: the instance is live, and this is the only shutdown path.
        let status = unsafe { ((*self.table).shutdown)(self.instance) };
        self.ledger
            .lock()
            .unwrap()
            .push("shutdown returned".to_owned());
        if status == MetisStatus::Ok {
            Ok(())
        } else {
            Err(map_status(status, "shutdown"))
        }
    }
}

impl Drop for AbiPlugin {
    fn drop(&mut self) {
        let _ = self.shutdown();
        // `library` drops after this body, and its own `Drop` records the
        // unload in the ledger, which is how the ordering is observable.
        let _ = &self.library;
    }
}

fn map_status(status: MetisStatus, capability: &str) -> PluginError {
    let detail = format!("the plugin returned {} for {capability}", status.as_str());
    match status {
        MetisStatus::InvalidArgument => PluginError::PluginFailed {
            status: Status::InvalidArgument,
            capability: capability.to_owned(),
            detail,
        },
        MetisStatus::NotFound => PluginError::PluginFailed {
            status: Status::NotFound,
            capability: capability.to_owned(),
            detail,
        },
        MetisStatus::PermissionDenied => PluginError::PluginFailed {
            status: Status::PermissionDenied,
            capability: capability.to_owned(),
            detail,
        },
        MetisStatus::Cancelled => PluginError::Cancelled,
        MetisStatus::Unsupported => PluginError::Unsupported {
            capability: capability.to_owned(),
            detail,
        },
        other => PluginError::PluginFailed {
            status: Status::Internal,
            capability: capability.to_owned(),
            detail: format!("the plugin returned {} for {capability}", other.as_str()),
        },
    }
}
