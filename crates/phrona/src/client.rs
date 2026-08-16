use std::net::IpAddr;
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
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36",
            ),
        );
        Self {
            profile: Profile::Chrome,
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
