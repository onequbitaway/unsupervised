//! The address of the client, and the rate limit for each address.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Instant;

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::limit::{limit_address, retry_after_secs};
use crate::state::{Counters, Gateway};

const FORWARDED_FOR: &str = "x-forwarded-for";

/// The address of the client that sent a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClientAddr(pub(crate) IpAddr);

impl<S: Send + Sync> FromRequestParts<S> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // The middleware below puts the address there. A router that runs
        // without it sees every client as one address.
        Ok(parts
            .extensions
            .get::<ClientAddr>()
            .copied()
            .unwrap_or(ClientAddr(IpAddr::V4(Ipv4Addr::UNSPECIFIED))))
    }
}

/// Finds the client address of a request.
///
/// With `hops` trusted proxies in front of the server, the client address
/// is the entry number `hops` from the right of `X-Forwarded-For`: each
/// trusted proxy appends the address that it got the request from. With
/// fewer entries than that, a proxy did not do its job, and the address
/// of the connection is used.
fn client_address(peer: Option<SocketAddr>, headers: &HeaderMap, hops: u8) -> IpAddr {
    let peer = peer.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |addr| addr.ip());
    if hops == 0 {
        return peer;
    }
    let entries: Vec<&str> = headers
        .get_all(FORWARDED_FOR)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();
    entries
        .len()
        .checked_sub(usize::from(hops))
        .and_then(|index| entries[index].parse::<IpAddr>().ok())
        .unwrap_or(peer)
}

/// Middleware: notes the client address and applies the rate limit of the
/// address.
pub(crate) async fn address_limit(
    State(gateway): State<Gateway>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|info| info.0);
    let address = client_address(peer, request.headers(), gateway.config().forwarded_hops);
    request.extensions_mut().insert(ClientAddr(address));
    if let Err(wait) = gateway.inner.address_limiter.check(limit_address(address), Instant::now()) {
        Counters::add(&gateway.inner.counters.rate_limited);
        return ApiError::rate_limited(retry_after_secs(wait)).into_response();
    }
    next.run(request).await
}
