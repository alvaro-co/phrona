use std::net::IpAddr;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use wreq::Uri;
use wreq::header::{HeaderMap, HeaderValue, USER_AGENT};
use wreq::redirect;
use wreq_util::Emulation;

use crate::error::{Error, Result};
use crate::extract::is_safe_ip;

/// Parse an IP literal from a URL/Uri host string, tolerating the brackets
/// that authority serialization adds around IPv6 addresses (`[::1]`).
fn parse_host_ip(host: &str) -> Option<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    if host.starts_with('[') && host.ends_with(']') {
        if let Ok(v6) = host[1..host.len() - 1].parse::<std::net::Ipv6Addr>() {
            return Some(IpAddr::V6(v6));
        }
    }
    None
}

/// Validate a target URL for SSRF safety: only `http`/`https` schemes, and
/// every address the hostname resolves to must pass [`is_safe_ip`]. Used for
/// the initial request (in `extract`) and for every redirect hop.
pub(crate) async fn validate_target(uri: &Uri) -> Result<()> {
    let scheme = uri.scheme_str().unwrap_or("");
    if scheme != "http" && scheme != "https" {
        return Err(Error::invalid_query(
            "client",
            "unsupported URL scheme (http/https only)",
        ));
    }
    let host = uri
        .host()
        .ok_or_else(|| Error::invalid_query("client", "URL has no host"))?;
    let port = uri
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    let safe = if let Some(ip) = parse_host_ip(host) {
        is_safe_ip(ip)
    } else {
        let addrs = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| Error::invalid_query("client", "host resolution failed"))?;
        addrs.into_iter().all(|sa| is_safe_ip(sa.ip()))
    };
    if safe {
        Ok(())
    } else {
        Err(Error::invalid_query(
            "client",
            "SSRF blocked: IP address is in a private/restricted range",
        ))
    }
}

/// Redirect policy that intercepts every hop: enforces the redirect limit
/// and validates scheme + destination IP before following. A non-`http(s)`
/// or private/restricted hop fails the request instead of being followed.
fn ssrf_redirect_policy(max_redirects: usize) -> redirect::Policy {
    redirect::Policy::custom(move |attempt| {
        attempt.pending(move |attempt| async move {
            if attempt.previous.len() > max_redirects {
                return attempt.error(Error::internal("client", "too many redirects"));
            }
            match validate_target(&attempt.uri).await {
                Ok(()) => attempt.follow(),
                Err(e) => attempt.error(e),
            }
        })
    })
}

/// Browser profile used to impersonate a real browser over TLS/HTTP2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Chrome,
    Chrome100,
    Chrome120,
    Chrome131,
    Chrome140,
    Chrome149,
    Firefox,
    Firefox139,
    Firefox148,
    Edge,
    Edge148,
    Safari,
    Safari26,
    Opera,
    Opera131,
    OkHttp,
    Random,
}

impl Profile {
    /// Resolve a lowercase profile name (family names and versioned
    /// variants as used by `phrona.yaml` / `PHRONA_ENGINES_PROFILE`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "chrome" | "chrome148" => Some(Profile::Chrome),
            "chrome149" => Some(Profile::Chrome149),
            "chrome140" => Some(Profile::Chrome140),
            "chrome131" => Some(Profile::Chrome131),
            "chrome120" => Some(Profile::Chrome120),
            "chrome100" => Some(Profile::Chrome100),
            "firefox" | "firefox148" => Some(Profile::Firefox),
            "firefox139" => Some(Profile::Firefox139),
            "safari" | "safari26" => Some(Profile::Safari),
            "edge" | "edge148" => Some(Profile::Edge),
            "opera" | "opera131" => Some(Profile::Opera),
            "okhttp" => Some(Profile::OkHttp),
            "random" => Some(Profile::Random),
            _ => None,
        }
    }

    fn to_emulation(self) -> Emulation {
        use wreq_util::Profile as P;
        let profile = match self {
            Profile::Chrome => P::Chrome148,
            Profile::Chrome100 => P::Chrome100,
            Profile::Chrome120 => P::Chrome120,
            Profile::Chrome131 => P::Chrome131,
            Profile::Chrome140 => P::Chrome140,
            Profile::Chrome149 => P::Chrome149,
            Profile::Firefox => P::Firefox148,
            Profile::Firefox139 => P::Firefox139,
            Profile::Firefox148 => P::Firefox148,
            Profile::Edge => P::Edge148,
            Profile::Edge148 => P::Edge148,
            Profile::Safari => P::Safari26,
            Profile::Safari26 => P::Safari26,
            Profile::Opera => P::Opera131,
            Profile::Opera131 => P::Opera131,
            Profile::OkHttp => P::OkHttp5,
            Profile::Random => return Emulation::random(),
        };
        Emulation::builder().profile(profile).build()
    }
}

/// Browser User-Agent strings matching the TLS/HTTP2 impersonation profiles
/// of [`Profile`]. Variants of a family (versioned profiles) use the family
/// UA; [`Profile::Random`] picks one at first use and caches it.
const UA_CHROME: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";
const UA_FIREFOX: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:148.0) Gecko/20100101 Firefox/148.0";
const UA_SAFARI: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.4 Safari/605.1.15";
const UA_EDGE: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36 Edg/148.0.0.0";
const UA_OPERA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36 OPR/134.0.0.0";
const UA_OKHTTP: &str = "okhttp/5.0.0-alpha.14";
const UA_POOL: [&str; 5] = [UA_CHROME, UA_FIREFOX, UA_SAFARI, UA_EDGE, UA_OPERA];

/// UA for a browser profile, matching the exact TLS impersonation family.
pub fn default_user_agent(profile: Profile) -> &'static str {
    match profile {
        Profile::Firefox | Profile::Firefox139 | Profile::Firefox148 => UA_FIREFOX,
        Profile::Safari | Profile::Safari26 => UA_SAFARI,
        Profile::Edge | Profile::Edge148 => UA_EDGE,
        Profile::Opera | Profile::Opera131 => UA_OPERA,
        Profile::OkHttp => UA_OKHTTP,
        // Random emulation: fall back to one of the known browser families.
        Profile::Random => RANDOM_UA.get_or_init(|| {
            use rand::Rng;
            let i = rand::rng().random_range(0..UA_POOL.len());
            UA_POOL[i]
        }),
        // Chrome and all versioned Chrome variants.
        Profile::Chrome
        | Profile::Chrome100
        | Profile::Chrome120
        | Profile::Chrome131
        | Profile::Chrome140
        | Profile::Chrome149 => UA_CHROME,
    }
}

static RANDOM_UA: OnceLock<&'static str> = OnceLock::new();

/// A sticky pool of persistent impersonated HTTP clients: one per proxy URL
/// (each with its own connection pool and cookie jar), or a single direct
/// client when no proxies are configured. [`ProxyPool::get_client`] assigns
/// clients round-robin; an engine task keeps its client for its whole
/// lifetime so multi-step flows (vqd -> i.js, sc -> search) stay pinned to
/// the same proxy and cookies.
pub struct ProxyPool {
    clients: Vec<HttpClient>,
    counter: AtomicUsize,
}

impl ProxyPool {
    /// Build one persistent client per proxy URL. An empty `proxies` list
    /// yields a single direct client.
    pub fn new(proxies: Vec<String>, profile: Profile, timeout: Duration) -> Result<Self> {
        let mut clients = Vec::with_capacity(proxies.len().max(1));
        for proxy in proxies {
            clients.push(
                HttpClient::builder()
                    .profile(profile)
                    .timeout(timeout)
                    .proxy(Some(proxy))
                    .build()?,
            );
        }
        if clients.is_empty() {
            clients.push(
                HttpClient::builder()
                    .profile(profile)
                    .timeout(timeout)
                    .build()?,
            );
        }
        Ok(Self {
            clients,
            counter: AtomicUsize::new(0),
        })
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Deterministic round-robin client selection.
    pub fn get_client(&self) -> &HttpClient {
        let idx = self.counter.fetch_add(1, Ordering::Relaxed) % self.clients.len();
        &self.clients[idx]
    }

    /// The first (or only) client — for non-engine flows such as `extract`.
    pub fn first(&self) -> &HttpClient {
        &self.clients[0]
    }
}

pub struct HttpClient {
    client: wreq::Client,
}

impl HttpClient {
    pub fn builder() -> HttpClientBuilder {
        HttpClientBuilder::default()
    }

    pub async fn get(&self, url: &str) -> Result<wreq::Response> {
        Ok(self.client.get(url).send().await?)
    }

    /// Single-hop GET with redirects disabled. The caller is responsible for
    /// following (and validating) any redirect itself — used by SSRF-guarded
    /// flows such as `extract`.
    pub async fn get_no_redirect(&self, url: &str) -> Result<wreq::Response> {
        Ok(self
            .client
            .get(url)
            .redirect(redirect::Policy::none())
            .send()
            .await?)
    }

    pub async fn get_with_headers(&self, url: &str, headers: &HeaderMap) -> Result<wreq::Response> {
        let mut rb = self.client.get(url);
        for (k, v) in headers {
            rb = rb.header(k, v);
        }
        Ok(rb.send().await?)
    }

    pub async fn post_form(&self, url: &str, form: &str) -> Result<wreq::Response> {
        Ok(self
            .client
            .post(url)
            .header(
                wreq::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form.to_string())
            .send()
            .await?)
    }

    pub async fn post_form_with_headers(
        &self,
        url: &str,
        form: &str,
        headers: &HeaderMap,
    ) -> Result<wreq::Response> {
        let mut rb = self
            .client
            .post(url)
            .header(
                wreq::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form.to_string());
        for (k, v) in headers {
            rb = rb.header(k, v);
        }
        Ok(rb.send().await?)
    }
}

pub struct HttpClientBuilder {
    profile: Profile,
    timeout: Duration,
    cookies: bool,
    redirects: usize,
    headers: HeaderMap,
    proxy: Option<String>,
}

impl Default for HttpClientBuilder {
    fn default() -> Self {
        let profile = Profile::Chrome;
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(default_user_agent(profile)),
        );
        Self {
            profile,
            timeout: Duration::from_secs(10),
            cookies: true,
            redirects: 10,
            headers,
            proxy: None,
        }
    }
}

impl HttpClientBuilder {
    pub fn profile(mut self, profile: Profile) -> Self {
        self.profile = profile;
        // keep the UA in lockstep with the TLS/HTTP2 impersonation profile
        self.headers.insert(
            USER_AGENT,
            HeaderValue::from_static(default_user_agent(profile)),
        );
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn proxy(mut self, proxy: Option<String>) -> Self {
        self.proxy = proxy;
        self
    }

    pub fn build(self) -> Result<HttpClient> {
        let mut builder = wreq::Client::builder()
            .emulation(self.profile.to_emulation())
            .timeout(self.timeout)
            .redirect(ssrf_redirect_policy(self.redirects));
        if self.cookies {
            builder = builder.cookie_store(true);
        }
        if !self.headers.is_empty() {
            builder = builder.default_headers(self.headers);
        }
        if let Some(proxy) = self.proxy {
            let p = wreq::Proxy::all(&proxy)
                .map_err(|_| Error::invalid_query("client", "invalid proxy URL"))?;
            builder = builder.proxy(p);
        }
        let client = builder
            .build()
            .map_err(|_| Error::internal("client", "client build failed"))?;
        Ok(HttpClient { client })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_pool_round_robin_is_deterministic() {
        let pool = ProxyPool::new(
            vec![
                "http://127.0.0.1:1".into(),
                "http://127.0.0.1:2".into(),
                "http://127.0.0.1:3".into(),
            ],
            Profile::Chrome,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(pool.len(), 3);
        let a = pool.get_client() as *const HttpClient;
        let b = pool.get_client() as *const HttpClient;
        let c = pool.get_client() as *const HttpClient;
        let a2 = pool.get_client() as *const HttpClient;
        let b2 = pool.get_client() as *const HttpClient;
        // strict rotation: a b c a b ...
        assert!(!std::ptr::eq(a, b));
        assert!(!std::ptr::eq(b, c));
        assert!(!std::ptr::eq(a, c));
        assert!(std::ptr::eq(a, a2));
        assert!(std::ptr::eq(b, b2));
    }

    #[test]
    fn proxy_pool_empty_yields_single_direct_client() {
        let pool = ProxyPool::new(vec![], Profile::Firefox, Duration::from_secs(5)).unwrap();
        assert_eq!(pool.len(), 1);
        assert!(std::ptr::eq(
            pool.get_client() as *const HttpClient,
            pool.get_client() as *const HttpClient,
        ));
        assert!(std::ptr::eq(pool.first(), pool.get_client()));
    }

    #[test]
    fn default_user_agent_matches_family() {
        assert!(default_user_agent(Profile::Firefox).contains("Firefox/"));
        assert!(default_user_agent(Profile::Firefox139).contains("Firefox/"));
        assert!(default_user_agent(Profile::Firefox148).contains("Firefox/"));
        assert!(default_user_agent(Profile::Safari).contains("Safari/"));
        assert!(default_user_agent(Profile::Safari26).contains("Version/"));
        assert!(default_user_agent(Profile::Chrome).contains("Chrome/"));
        assert!(default_user_agent(Profile::Chrome100).contains("Chrome/"));
        assert!(default_user_agent(Profile::Edge).contains("Edg/"));
        assert!(default_user_agent(Profile::Opera).contains("OPR/"));
        assert!(default_user_agent(Profile::OkHttp).starts_with("okhttp/"));
        let r1 = default_user_agent(Profile::Random);
        let r2 = default_user_agent(Profile::Random);
        assert_eq!(r1, r2, "random UA is cached");
        assert!(UA_POOL.contains(&r1));
    }
}
