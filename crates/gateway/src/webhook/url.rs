//! Which addresses a webhook can point at.
//!
//! A webhook makes the server send a request to an address that a client
//! chose. Without a check, a client could aim the server at its own
//! network: the metadata service of a cloud, a database, the loopback
//! interface. So the server takes only `http` and `https` addresses and,
//! outside development, only hosts with public addresses. The check runs
//! when the webhook is set and again for every connection, because a
//! name can change the address that it points at.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use reqwest::Url;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::config::WebhookConfig;

/// Returns true when an address is on the public internet: not loopback,
/// not private, not link-local, not reserved for documentation or tests,
/// and not multicast.
#[must_use]
pub fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    let special = a == 0                                  // "this network"
        || a == 10                                        // private
        || (a == 100 && (64..128).contains(&b))           // shared address space
        || a == 127                                       // loopback
        || (a == 169 && b == 254)                         // link-local, cloud metadata
        || (a == 172 && (16..32).contains(&b))            // private
        || (a == 192 && b == 0 && c == 0)                 // protocol assignments
        || (a == 192 && b == 0 && c == 2)                 // documentation
        || (a == 192 && b == 168)                         // private
        || (a == 198 && (18..20).contains(&b))            // benchmarks
        || (a == 198 && b == 51 && c == 100)              // documentation
        || (a == 203 && b == 0 && c == 113)               // documentation
        || a >= 224; // multicast, reserved, broadcast
    !special
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    // Forms that carry an IPv4 address are as public as that address.
    if let Some(v4) = address.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let embedded = |high: u16, low: u16| {
        let [a, b] = high.to_be_bytes();
        let [c, d] = low.to_be_bytes();
        Ipv4Addr::new(a, b, c, d)
    };
    match segments {
        // `::`, `::1` and the old IPv4-compatible form (::a.b.c.d).
        [0, 0, 0, 0, 0, 0, _, _] => return false,
        // NAT64 (64:ff9b::/96).
        [0x64, 0xff9b, 0, 0, 0, 0, high, low] => return is_public_v4(embedded(high, low)),
        // 6to4 (2002::/16).
        [0x2002, high, low, ..] => return is_public_v4(embedded(high, low)),
        _ => {}
    }
    let first = segments[0];
    let special = (first & 0xfe00) == 0xfc00               // unique local
        || (first & 0xffc0) == 0xfe80                      // link-local
        || (first & 0xffc0) == 0xfec0                      // site-local (old)
        || (first & 0xff00) == 0xff00                      // multicast
        || (first == 0x2001 && segments[1] == 0x0db8); // documentation
    !special
}

/// Reads the host of an address as an IP address, when it is one.
fn literal_host(url: &Url) -> Option<IpAddr> {
    let host = url.host_str()?;
    host.trim_start_matches('[').trim_end_matches(']').parse().ok()
}

/// Checks the form of a webhook address: the length, the scheme, the
/// host, and that it holds no user name or password. A host that is an IP
/// address must be public, unless the configuration permits private ones.
///
/// # Errors
///
/// A sentence that says what is wrong.
pub(crate) fn parse(text: &str, config: &WebhookConfig) -> Result<Url, String> {
    if text.len() > config.max_url_bytes {
        return Err(format!("`url` has at most {} bytes.", config.max_url_bytes));
    }
    let url = Url::parse(text).map_err(|error| format!("`url` is not an address: {error}."))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("`url` must start with `http://` or `https://`.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("`url` must not hold a user name or a password.".into());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("`url` has no host.".into());
    }
    if let Some(address) = literal_host(&url)
        && !config.allow_private
        && !is_public_address(address)
    {
        return Err(private_refusal());
    }
    Ok(url)
}

fn private_refusal() -> String {
    "`url` points at a private, loopback or reserved address. The server sends webhooks only \
     to public addresses."
        .into()
}

/// Checks a webhook address fully: its form, and that every address of
/// its host is public.
///
/// # Errors
///
/// A sentence that says what is wrong.
pub(crate) async fn check(text: &str, config: &WebhookConfig) -> Result<Url, String> {
    let url = parse(text, config)?;
    if config.allow_private || literal_host(&url).is_some() {
        return Ok(url);
    }
    let host = url.host_str().unwrap_or_default().to_string();
    let port = url.port_or_known_default().unwrap_or(0);
    let lookup = tokio::net::lookup_host((host, port));
    let addresses: Vec<SocketAddr> = match tokio::time::timeout(config.timeout(), lookup).await {
        Ok(Ok(addresses)) => addresses.collect(),
        Ok(Err(_)) | Err(_) => Vec::new(),
    };
    if addresses.is_empty() {
        return Err("The host of `url` has no address. Check the name.".into());
    }
    if addresses.iter().any(|address| !is_public_address(address.ip())) {
        return Err(private_refusal());
    }
    Ok(url)
}

/// Resolves host names for deliveries and leaves out every address that
/// is not public. A name that changes its address after the webhook was
/// set cannot lead the server into a private network.
struct PublicOnly {
    allow_private: bool,
}

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_private = self.allow_private;
        Box::pin(async move {
            let found = tokio::net::lookup_host((name.as_str(), 0)).await?;
            let usable: Vec<SocketAddr> =
                found.filter(|a| allow_private || is_public_address(a.ip())).collect();
            if usable.is_empty() {
                return Err("the host has no public address".into());
            }
            Ok(Box::new(usable.into_iter()) as Addrs)
        })
    }
}

/// Builds the HTTP client for deliveries: a time limit, no redirects, no
/// proxy, and the resolver above.
pub(crate) fn client(config: &WebhookConfig) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(config.timeout())
        .connect_timeout(config.timeout())
        // A redirect could point at a private address.
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .dns_resolver(Arc::new(PublicOnly { allow_private: config.allow_private }))
        .user_agent(concat!("unsupervised-webhook/", env!("CARGO_PKG_VERSION")))
        .build()
}
