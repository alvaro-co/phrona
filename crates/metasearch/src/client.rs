use std::time::Duration;

use wreq::header::{HeaderMap, HeaderValue, USER_AGENT};
use wreq::redirect;
use wreq_util::Emulation;

use crate::error::{Error, Result};

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
        self.client.get(url).send().await.map_err(map_err)
    }

    pub async fn get_with_headers(&self, url: &str, headers: &HeaderMap) -> Result<wreq::Response> {
        let mut rb = self.client.get(url);
        for (k, v) in headers {
            rb = rb.header(k, v);
        }
        rb.send().await.map_err(map_err)
    }

    pub async fn post_form(&self, url: &str, form: &str) -> Result<wreq::Response> {
        self.client
            .post(url)
            .header(
                wreq::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form.to_string())
            .send()
            .await
            .map_err(map_err)
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
        rb.send().await.map_err(map_err)
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

    pub fn cookies(mut self, enable: bool) -> Self {
        self.cookies = enable;
        self
    }

    pub fn redirects(mut self, max: usize) -> Self {
        self.redirects = max;
        self
    }

    pub fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers
            .insert(name, HeaderValue::from_str(value).unwrap());
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
            .redirect(redirect::Policy::limited(self.redirects));
        if self.cookies {
            builder = builder.cookie_store(true);
        }
        if !self.headers.is_empty() {
            builder = builder.default_headers(self.headers);
        }
        if let Some(proxy) = self.proxy {
            let p = wreq::Proxy::all(&proxy).map_err(|e| Error::Request(e.to_string()))?;
            builder = builder.proxy(p);
        }
        let client = builder.build().map_err(|e| Error::Http(e.to_string()))?;
        Ok(HttpClient { client })
    }
}

fn map_err(e: wreq::Error) -> Error {
    let msg = e.to_string();
    if msg.contains("timed out") || msg.contains("timeout") {
        Error::Timeout(msg)
    } else {
        Error::Http(msg)
    }
}
