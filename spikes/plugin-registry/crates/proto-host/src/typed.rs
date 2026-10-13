//! PROTOTYPE (throwaway). Not product code.
//!
//! Typed method families above the one erased seam.
//!
//! This is where the prototype shows what type erasure costs. `Toolset` and
//! `ProviderSet` are ordinary typed Rust facades: their signatures name real
//! request and response types, and a caller gets compile time checking at the
//! call site. Underneath, every method is a three line delegation into
//! `PluginRegistry::invoke`, which takes a `&str` and `Payload` and returns
//! `Result<Payload, PluginError>`. Nothing about the typing survives the seam;
//! the route knows a `&str` and a compiled schema, not a Rust type.

use std::marker::PhantomData;
use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;

use proto_registry::{Call, CapabilityKind, Payload, PluginError, PluginRegistry, block_on};

use crate::types::{
    AddRequest, AddResponse, BadResponse, BadResponseRequest, CompleteRequest, CompleteResponse,
    EchoRequest, EchoResponse, MismatchedEchoRequest, SummarizeRequest, SummarizeResponse,
    UppercaseRequest, UppercaseResponse, WordCountRequest, WordCountResponse,
};

/// A typed client for one capability: the request and response types are named
/// at the call site, the seam underneath sees bytes.
pub struct TypedHandle<Req, Resp> {
    registry: Arc<PluginRegistry>,
    key: String,
    kind: CapabilityKind,
    _types: PhantomData<fn(&Req) -> Resp>,
}

pub type ToolHandle<Req, Resp> = TypedHandle<Req, Resp>;
pub type ProviderHandle<Req, Resp> = TypedHandle<Req, Resp>;

impl<Req, Resp> TypedHandle<Req, Resp>
where
    Req: Serialize,
    Resp: DeserializeOwned,
{
    pub fn call(&self, request: &Req) -> Result<Resp, PluginError> {
        let call = Call::new();
        let payload = Payload::from_json(request)?;
        let response = block_on(self.registry.invoke(&self.key, payload, &call))?;
        response.to_json()
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn kind(&self) -> CapabilityKind {
        self.kind
    }
}

fn handle_for<Req, Resp>(
    registry: &Arc<PluginRegistry>,
    key: &str,
    kind: CapabilityKind,
) -> Result<TypedHandle<Req, Resp>, PluginError> {
    let route = registry.route(key).ok_or_else(|| PluginError::CapabilityNotFound {
        capability: key.to_owned(),
    })?;
    if route.kind() != kind {
        return Err(PluginError::Unsupported {
            capability: key.to_owned(),
            detail: format!("capability {key} is a {:?}, not a {:?}", route.kind(), kind),
        });
    }
    Ok(TypedHandle {
        registry: Arc::clone(registry),
        key: key.to_owned(),
        kind,
        _types: PhantomData,
    })
}

/// The tool method family.
pub struct Toolset {
    registry: Arc<PluginRegistry>,
}

impl Toolset {
    pub fn new(registry: Arc<PluginRegistry>) -> Self {
        Toolset { registry }
    }

    pub fn handle<Req, Resp>(&self, key: &str) -> Result<ToolHandle<Req, Resp>, PluginError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        handle_for(&self.registry, key, CapabilityKind::Tool)
    }

    pub fn echo(&self, request: &EchoRequest) -> Result<EchoResponse, PluginError> {
        self.handle::<EchoRequest, EchoResponse>("tool.echo")?
            .call(request)
    }

    pub fn echo_mismatched(
        &self,
        request: &MismatchedEchoRequest,
    ) -> Result<EchoResponse, PluginError> {
        self.handle::<MismatchedEchoRequest, EchoResponse>("tool.echo")?
            .call(request)
    }

    pub fn add(&self, request: &AddRequest) -> Result<AddResponse, PluginError> {
        self.handle::<AddRequest, AddResponse>("tool.add")?
            .call(request)
    }

    pub fn uppercase(&self, request: &UppercaseRequest) -> Result<UppercaseResponse, PluginError> {
        self.handle::<UppercaseRequest, UppercaseResponse>("tool.uppercase")?
            .call(request)
    }

    pub fn word_count(
        &self,
        request: &WordCountRequest,
    ) -> Result<WordCountResponse, PluginError> {
        self.handle::<WordCountRequest, WordCountResponse>("tool.word_count")?
            .call(request)
    }

    pub fn bad_response(&self, request: &BadResponseRequest) -> Result<BadResponse, PluginError> {
        self.handle::<BadResponseRequest, BadResponse>("tool.bad_response")?
            .call(request)
    }
}

/// The provider method family. Same shape, different kind.
pub struct ProviderSet {
    registry: Arc<PluginRegistry>,
}

impl ProviderSet {
    pub fn new(registry: Arc<PluginRegistry>) -> Self {
        ProviderSet { registry }
    }

    pub fn handle<Req, Resp>(&self, key: &str) -> Result<ProviderHandle<Req, Resp>, PluginError>
    where
        Req: Serialize,
        Resp: DeserializeOwned,
    {
        handle_for(&self.registry, key, CapabilityKind::Provider)
    }

    pub fn complete(&self, request: &CompleteRequest) -> Result<CompleteResponse, PluginError> {
        self.handle::<CompleteRequest, CompleteResponse>("provider.complete")?
            .call(request)
    }

    pub fn summarize(&self, request: &SummarizeRequest) -> Result<SummarizeResponse, PluginError> {
        self.handle::<SummarizeRequest, SummarizeResponse>("provider.summarize")?
            .call(request)
    }
}
