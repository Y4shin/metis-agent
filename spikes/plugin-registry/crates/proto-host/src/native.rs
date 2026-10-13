//! PROTOTYPE (throwaway). Not product code.
//!
//! The native in-process plugins, behind the same `Plugin` seam the ABI adapter
//! implements. No ABI, no buffers, no loading: one Rust type.
//!
//! They exist to prove two things round one did not: that the registry's
//! first-class lifecycle works against a plugin that is not a cdylib (disable,
//! enable, unregister), and that the new fixes have teeth (a listing that fails,
//! a manifest the host rejects, a name that collides).

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use proto_registry::{
    BoxFuture, Call, CapabilityDescriptor, CapabilityKind, Payload, Plugin, PluginError,
    PluginManifest,
};

use crate::types::{
    self, EchoRequest, EchoResponse, SummarizeRequest, SummarizeResponse, ThrowawayRequest,
    ThrowawayResponse, WordCountRequest, WordCountResponse,
};

fn descriptor(
    plugin: &str,
    name: &'static str,
    kind: CapabilityKind,
    version: u32,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: name.to_owned(),
        kind,
        version,
        request_schema_id: types::schema_id(plugin, name, "request"),
        response_schema_id: types::schema_id(plugin, name, "response"),
    }
}

/// The native plugin's typed method family. Nothing here knows about the seam.
pub struct NativeTextTools;

impl NativeTextTools {
    pub fn word_count(&self, request: WordCountRequest) -> Result<WordCountResponse, PluginError> {
        if request.text.trim().is_empty() {
            return Err(PluginError::PluginFailed {
                status: proto_registry::Status::InvalidArgument,
                capability: "tool.word_count".to_owned(),
                detail: "text must not be empty".to_owned(),
            });
        }
        Ok(WordCountResponse {
            words: request.text.split_whitespace().count() as u64,
            chars: request.text.chars().count() as u64,
        })
    }

    pub fn summarize(&self, request: SummarizeRequest) -> Result<SummarizeResponse, PluginError> {
        let first = request
            .text
            .split_whitespace()
            .next()
            .unwrap_or("(empty)")
            .to_owned();
        Ok(SummarizeResponse {
            summary: format!("{first}..."),
            source: "native-text-tools".to_owned(),
        })
    }
}

impl Plugin for NativeTextTools {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "native-text-tools".to_owned(),
            version: "0.1.0".to_owned(),
            abi_major: 1,
        }
    }

    fn capabilities(&self) -> Result<Vec<CapabilityDescriptor>, PluginError> {
        let id = "native-text-tools";
        Ok(vec![
            descriptor(id, "tool.word_count", CapabilityKind::Tool, 1),
            descriptor(id, "provider.summarize", CapabilityKind::Provider, 1),
        ])
    }

    fn schemas(&self, ids: &[String]) -> Result<BTreeMap<String, Vec<u8>>, PluginError> {
        let mut out = BTreeMap::new();
        for id in ids {
            let value: Option<Value> = if id.ends_with("tool.word_count.request") {
                Some(types::schema_of::<WordCountRequest>())
            } else if id.ends_with("tool.word_count.response") {
                Some(types::schema_of::<WordCountResponse>())
            } else if id.ends_with("provider.summarize.request") {
                Some(types::schema_of::<SummarizeRequest>())
            } else if id.ends_with("provider.summarize.response") {
                Some(types::schema_of::<SummarizeResponse>())
            } else {
                None
            };
            if let Some(value) = value {
                out.insert(id.clone(), serde_json::to_vec(&value).unwrap_or_default());
            }
        }
        Ok(out)
    }

    fn invoke<'a>(
        &'a self,
        capability: &'a str,
        payload: Payload,
        _call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>> {
        Box::pin(async move {
            match capability {
                "tool.word_count" => {
                    let request: WordCountRequest = payload.to_json()?;
                    let response = self.word_count(request)?;
                    Payload::from_json(&response)
                }
                "provider.summarize" => {
                    let request: SummarizeRequest = payload.to_json()?;
                    let response = self.summarize(request)?;
                    Payload::from_json(&response)
                }
                other => Err(PluginError::CapabilityNotFound {
                    capability: other.to_owned(),
                }),
            }
        })
    }

    fn cancel(&self, _request_id: u64) -> Result<(), PluginError> {
        Err(PluginError::PluginFailed {
            status: proto_registry::Status::NotFound,
            capability: "cancel".to_owned(),
            detail: "native-text-tools completes inline and has nothing pending".to_owned(),
        })
    }

    fn shutdown(&self) -> Result<(), PluginError> {
        Ok(())
    }
}

/// A native plugin claiming a capability name the cdylib already owns, so the
/// global uniqueness rule has something to refuse and the namespacing rule has
/// something to admit.
pub struct DuplicateEchoTools;

impl Plugin for DuplicateEchoTools {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "duplicate-echo-tools".to_owned(),
            version: "0.1.0".to_owned(),
            abi_major: 1,
        }
    }

    fn capabilities(&self) -> Result<Vec<CapabilityDescriptor>, PluginError> {
        Ok(vec![descriptor(
            "duplicate-echo-tools",
            "tool.echo",
            CapabilityKind::Tool,
            1,
        )])
    }

    fn schemas(&self, ids: &[String]) -> Result<BTreeMap<String, Vec<u8>>, PluginError> {
        let mut out = BTreeMap::new();
        for id in ids {
            let value = if id.ends_with("tool.echo.request") {
                types::schema_of::<EchoRequest>()
            } else {
                types::schema_of::<EchoResponse>()
            };
            out.insert(id.clone(), serde_json::to_vec(&value).unwrap_or_default());
        }
        Ok(out)
    }

    fn invoke<'a>(
        &'a self,
        capability: &'a str,
        payload: Payload,
        _call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>> {
        Box::pin(async move {
            if capability != "tool.echo" {
                return Err(PluginError::CapabilityNotFound {
                    capability: capability.to_owned(),
                });
            }
            let request: EchoRequest = payload.to_json()?;
            // A distinct answer, so the namespacing check can tell whose
            // capability answered.
            Payload::from_json(&json!({
                "echo": format!("duplicate:{message}", message = request.message),
                "length": request.message.len() as u64,
            }))
        })
    }

    fn cancel(&self, _request_id: u64) -> Result<(), PluginError> {
        Ok(())
    }

    fn shutdown(&self) -> Result<(), PluginError> {
        Ok(())
    }
}

/// A plugin whose capability discovery fails. Registering it must fail with a
/// typed discovery error and register nothing (round one's issue 1).
pub struct BrokenDiscovery;

impl Plugin for BrokenDiscovery {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "broken-discovery".to_owned(),
            version: "0.1.0".to_owned(),
            abi_major: 1,
        }
    }

    fn capabilities(&self) -> Result<Vec<CapabilityDescriptor>, PluginError> {
        Err(PluginError::Discovery {
            plugin: "broken-discovery".to_owned(),
            detail: "the capability listing backend is unavailable".to_owned(),
        })
    }

    fn schemas(&self, _ids: &[String]) -> Result<BTreeMap<String, Vec<u8>>, PluginError> {
        unreachable!("discovery fails before schemas are fetched")
    }

    fn invoke<'a>(
        &'a self,
        _capability: &'a str,
        _payload: Payload,
        _call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>> {
        Box::pin(async move {
            Err(PluginError::CapabilityNotFound {
                capability: "broken-discovery".to_owned(),
            })
        })
    }

    fn cancel(&self, _request_id: u64) -> Result<(), PluginError> {
        Ok(())
    }

    fn shutdown(&self) -> Result<(), PluginError> {
        Ok(())
    }
}

/// A plugin that targets another ABI major, so the manifest check has a
/// failure to report.
pub struct WrongAbi;

impl Plugin for WrongAbi {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "wrong-abi".to_owned(),
            version: "0.1.0".to_owned(),
            abi_major: 2,
        }
    }

    fn capabilities(&self) -> Result<Vec<CapabilityDescriptor>, PluginError> {
        Ok(vec![descriptor(
            "wrong-abi",
            "tool.wrong_abi",
            CapabilityKind::Tool,
            1,
        )])
    }

    fn schemas(&self, ids: &[String]) -> Result<BTreeMap<String, Vec<u8>>, PluginError> {
        let mut out = BTreeMap::new();
        for id in ids {
            out.insert(id.clone(), b"{}".to_vec());
        }
        Ok(out)
    }

    fn invoke<'a>(
        &'a self,
        _capability: &'a str,
        _payload: Payload,
        _call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>> {
        Box::pin(async move {
            Err(PluginError::CapabilityNotFound {
                capability: "wrong-abi".to_owned(),
            })
        })
    }

    fn cancel(&self, _request_id: u64) -> Result<(), PluginError> {
        Ok(())
    }

    fn shutdown(&self) -> Result<(), PluginError> {
        Ok(())
    }
}

/// A plugin that exists only to be registered and then unregistered.
pub struct Throwaway;

impl Plugin for Throwaway {
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "throwaway".to_owned(),
            version: "0.1.0".to_owned(),
            abi_major: 1,
        }
    }

    fn capabilities(&self) -> Result<Vec<CapabilityDescriptor>, PluginError> {
        Ok(vec![descriptor(
            "throwaway",
            "tool.throwaway",
            CapabilityKind::Tool,
            1,
        )])
    }

    fn schemas(&self, ids: &[String]) -> Result<BTreeMap<String, Vec<u8>>, PluginError> {
        let mut out = BTreeMap::new();
        for id in ids {
            let value = if id.ends_with("tool.throwaway.request") {
                types::schema_of::<ThrowawayRequest>()
            } else {
                types::schema_of::<ThrowawayResponse>()
            };
            out.insert(id.clone(), serde_json::to_vec(&value).unwrap_or_default());
        }
        Ok(out)
    }

    fn invoke<'a>(
        &'a self,
        capability: &'a str,
        payload: Payload,
        _call: &'a Call,
    ) -> BoxFuture<'a, Result<Payload, PluginError>> {
        Box::pin(async move {
            if capability != "tool.throwaway" {
                return Err(PluginError::CapabilityNotFound {
                    capability: capability.to_owned(),
                });
            }
            let request: ThrowawayRequest = payload.to_json()?;
            Payload::from_json(&ThrowawayResponse {
                doubled: request.value * 2,
            })
        })
    }

    fn cancel(&self, _request_id: u64) -> Result<(), PluginError> {
        Ok(())
    }

    fn shutdown(&self) -> Result<(), PluginError> {
        Ok(())
    }
}

/// The native plugins behind the seam.
pub fn native_text_tools() -> Arc<dyn Plugin> {
    Arc::new(NativeTextTools)
}
pub fn duplicate_echo_tools() -> Arc<dyn Plugin> {
    Arc::new(DuplicateEchoTools)
}
pub fn broken_discovery() -> Arc<dyn Plugin> {
    Arc::new(BrokenDiscovery)
}
pub fn wrong_abi() -> Arc<dyn Plugin> {
    Arc::new(WrongAbi)
}
pub fn throwaway() -> Arc<dyn Plugin> {
    Arc::new(Throwaway)
}
