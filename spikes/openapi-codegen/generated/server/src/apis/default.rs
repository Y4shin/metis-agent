use async_trait::async_trait;
use axum::extract::*;
use axum_extra::extract::{CookieJar, Multipart};
use bytes::Bytes;
use http::Method;
use serde::{Deserialize, Serialize};

use crate::{models, types::*};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[must_use]
#[allow(clippy::large_enum_variant)]
pub enum EchoResponse {
    /// Echoed message.
    Status200_EchoedMessage
    (models::EchoResponse)
    ,
    /// Bad request.
    Status400_BadRequest
    (models::ApiError)
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[must_use]
#[allow(clippy::large_enum_variant)]
pub enum StreamEventsResponse {
    /// A stream of events.
    Status200_AStreamOfEvents
    (String)
}


/// Default
#[async_trait]
#[allow(clippy::ptr_arg)]
pub trait Default {
    /// Echo a message back with a server timestamp..
    ///
    /// Echo - POST /v1/echo
    async fn echo(
    &self,
    method: Method,
    host: Host,
    cookies: CookieJar,
            body: models::EchoRequest,
    ) -> Result<EchoResponse, ()>;

    /// Server-sent event stream with resumable cursor..
    ///
    /// StreamEvents - GET /v1/events
    async fn stream_events(
    &self,
    method: Method,
    host: Host,
    cookies: CookieJar,
      header_params: models::StreamEventsHeaderParams,
    ) -> Result<StreamEventsResponse, ()>;
}
