//! Structured errors with scope and kind classification.

use std::fmt;
use std::time::Duration;

/// The layer of the stack a failure belongs to. This lets the orchestrator
/// and callers react differently to egress blocks (disable/switch the
/// engine), provider outages (retry later), schema drift (alert the parser)
/// and query problems (fix the request).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorScope {
    /// Egress IP blocked, TLS fingerprint flagged, proxy/tunnel failure.
    Egress,
    /// Upstream provider is down, global 5xx, true 429 rate limit.
    Provider,
    /// Upstream DOM or JSON schema mutated; a parser failed.
    Schema,
    /// The request itself is invalid (bad query, no engines, ...).
    Query,
    /// A local/internal failure (I/O, client construction, ...).
    Internal,
}

/// The physical, observable failure behind an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// True HTTP 429 with an optional `Retry-After` hint.
    RateLimited {
        /// Optional seconds to wait before retrying, from `Retry-After`.
        retry_after: Option<Duration>,
    },
    /// Blocked by an anti-bot system.
    Blocked(BlockDetails),
    /// The response deviates from the expected schema and could not be
    /// parsed (DOM/Schema mutation, wrong content type, invalid JSON).
    MalformedPayload {
        /// Static description of the deviation.
        context: &'static str,
    },
    /// Upstream returned a non-2xx error status.
    UpstreamUnavailable {
        /// The HTTP status code returned.
        status: u16,
    },
    /// Every engine failed; the search as a whole produced nothing. Carries
    /// a short per-engine summary (`name: error`) for diagnostics.
    AllProvidersFailed {
        /// Per-engine `name: error` summaries.
        details: Vec<String>,
    },
    /// The request timed out.
    Timeout,
    /// The network failed (connect error, reset, TLS, ...).
    NetworkFailure,
    /// The request is invalid.
    InvalidQuery {
        /// Static description of what is invalid.
        context: &'static str,
    },
    /// A local/internal failure.
    Internal {
        /// Static description of the failure.
        context: &'static str,
    },
}

/// The specific anti-bot system that blocked a request, carried by
/// [`ErrorKind::Blocked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockDetails {
    /// Challenge page served by Cloudflare.
    Cloudflare,
    /// A CAPTCHA was required.
    Captcha,
    /// The egress IP was banned.
    IpBan,
    /// Generic bot detection (rate-based or fingerprinting).
    BotDetection,
}

/// Structured error: the observable failure (`kind`) plus the layer it
/// belongs to (`scope`), the producing `engine`, the HTTP status when
/// known, and an optional static message. Construction and Display are
/// allocation-free on every path except `AllProvidersFailed`, which carries
/// a per-engine diagnostic summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// The layer of the stack the failure belongs to.
    pub scope: ErrorScope,
    /// The observable failure.
    pub kind: ErrorKind,
    /// Producing engine, or `"client"` / `"orchestrator"` for non-engine
    /// failures.
    pub engine: &'static str,
    /// HTTP status when the failure carried one.
    pub http_status: Option<u16>,
    /// Optional static message.
    pub message: Option<&'static str>,
}

impl Error {
    /// True 429 (optionally carrying `Retry-After`).
    pub fn rate_limited(engine: &'static str, retry_after: Option<Duration>) -> Self {
        Self {
            scope: ErrorScope::Provider,
            kind: ErrorKind::RateLimited { retry_after },
            engine,
            http_status: Some(429),
            message: None,
        }
    }

    /// Blocked by an anti-bot system.
    pub fn blocked(engine: &'static str, details: BlockDetails) -> Self {
        Self {
            scope: ErrorScope::Egress,
            kind: ErrorKind::Blocked(details),
            engine,
            http_status: None,
            message: None,
        }
    }

    /// The response deviated from the expected schema.
    pub fn schema(engine: &'static str, context: &'static str) -> Self {
        Self {
            scope: ErrorScope::Schema,
            kind: ErrorKind::MalformedPayload { context },
            engine,
            http_status: None,
            message: None,
        }
    }

    /// Upstream returned a non-2xx error status.
    pub fn unavailable(engine: &'static str, status: u16) -> Self {
        Self {
            scope: ErrorScope::Provider,
            kind: ErrorKind::UpstreamUnavailable { status },
            engine,
            http_status: Some(status),
            message: None,
        }
    }

    /// The request timed out.
    pub fn timeout(engine: &'static str) -> Self {
        Self {
            scope: ErrorScope::Egress,
            kind: ErrorKind::Timeout,
            engine,
            http_status: None,
            message: None,
        }
    }

    /// The network failed at the transport level.
    pub fn network(engine: &'static str) -> Self {
        Self {
            scope: ErrorScope::Egress,
            kind: ErrorKind::NetworkFailure,
            engine,
            http_status: None,
            message: None,
        }
    }

    /// The request itself is invalid.
    pub fn invalid_query(engine: &'static str, context: &'static str) -> Self {
        Self {
            scope: ErrorScope::Query,
            kind: ErrorKind::InvalidQuery { context },
            engine,
            http_status: None,
            message: None,
        }
    }

    /// A local/internal failure.
    pub fn internal(engine: &'static str, context: &'static str) -> Self {
        Self {
            scope: ErrorScope::Internal,
            kind: ErrorKind::Internal { context },
            engine,
            http_status: None,
            message: None,
        }
    }

    /// Every engine failed for a search.
    pub fn all_failed(engine: &'static str, details: Vec<String>) -> Self {
        Self {
            scope: ErrorScope::Provider,
            kind: ErrorKind::AllProvidersFailed { details },
            engine,
            http_status: None,
            message: None,
        }
    }

    /// The layer of the stack the error belongs to ([`ErrorScope`]).
    pub fn scope(&self) -> ErrorScope {
        self.scope
    }

    /// The observable failure ([`ErrorKind`]).
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Fixed SSRF-guard contexts. These double as the recovery vocabulary
    /// for [`From<wreq::Error>`]: a policy refusal boxed by the transport
    /// is recognized by its context and rebuilt with its classification.
    /// Private/loopback/link-local destination refused.
    pub const SSRF_PRIVATE_IP: &'static str =
        "SSRF blocked: IP address is in a private/restricted range";
    /// Destination refused by the domain allow/deny policy.
    pub const SSRF_DOMAIN_POLICY: &'static str =
        "target host is blocked by the domain allow/deny policy";
    /// Non-`http(s)` URL scheme refused.
    pub const SSRF_SCHEME: &'static str = "unsupported URL scheme (http/https only)";
    /// URL without a host refused.
    pub const SSRF_NO_HOST: &'static str = "URL has no host";
    /// Hostname did not resolve.
    pub const SSRF_DNS: &'static str = "host resolution failed";

    /// Rebuild a policy refusal from a transport-boxed Display string, if
    /// it carries one of the fixed [`Error::SSRF_*`] contexts.
    fn from_policy_display(display: &str) -> Option<Self> {
        let context = [
            Self::SSRF_PRIVATE_IP,
            Self::SSRF_DOMAIN_POLICY,
            Self::SSRF_SCHEME,
            Self::SSRF_NO_HOST,
            Self::SSRF_DNS,
        ]
        .into_iter()
        .find(|c| display.contains(c))?;
        Some(Error::invalid_query("client", context))
    }

    /// Whether this failure may be fixed by a fresh browser session: an
    /// anti-bot block or a transport failure, both of which the silent
    /// session-refresh path knows how to retry. Used by the orchestrator
    /// to pick refresh candidates without string-matching labels.
    pub fn may_require_session(&self) -> bool {
        self.kind.may_require_session()
    }
}

impl ErrorKind {
    /// See [`Error::may_require_session`].
    pub fn may_require_session(&self) -> bool {
        matches!(self, ErrorKind::Blocked(_) | ErrorKind::NetworkFailure)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::RateLimited { retry_after } => match retry_after {
                Some(d) => write!(f, "rate limited (retry after {}s)", d.as_secs()),
                None => write!(f, "rate limited"),
            },
            ErrorKind::Blocked(d) => write!(f, "blocked ({d:?})"),
            ErrorKind::MalformedPayload { context } => write!(f, "malformed payload: {context}"),
            ErrorKind::UpstreamUnavailable { status } => {
                write!(f, "upstream unavailable (status {status})")
            }
            ErrorKind::AllProvidersFailed { details } => {
                if details.is_empty() {
                    write!(f, "all search providers failed")
                } else {
                    write!(f, "all search providers failed: {}", details.join("; "))
                }
            }
            ErrorKind::Timeout => write!(f, "timeout"),
            ErrorKind::NetworkFailure => write!(f, "network failure"),
            ErrorKind::InvalidQuery { context } => write!(f, "invalid query: {context}"),
            ErrorKind::Internal { context } => write!(f, "internal error: {context}"),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)?;
        write!(f, " [scope={:?}, engine={}]", self.scope, self.engine)?;
        if let Some(status) = self.http_status {
            write!(f, " [status={status}]")?;
        }
        if let Some(m) = self.message {
            write!(f, ": {m}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        // `context` is static, so map the I/O kind to a fixed label
        // instead of dropping it.
        use std::io::ErrorKind as IoKind;
        let context = match e.kind() {
            IoKind::NotFound => "file not found",
            IoKind::PermissionDenied => "permission denied",
            IoKind::ConnectionRefused => "connection refused",
            IoKind::ConnectionReset | IoKind::BrokenPipe => "connection reset",
            IoKind::TimedOut => "i/o timed out",
            _ => "i/o failure",
        };
        Error::internal("io", context)
    }
}

impl From<wreq::Error> for Error {
    fn from(e: wreq::Error) -> Self {
        // Policy errors from the SSRF redirect guard (our own `Error`)
        // round-trip through wreq boxed, and `dyn Error` cannot be
        // downcast on stable. Recover them by their fixed contexts so a
        // blocked redirect hop keeps its classification instead of
        // degrading to a generic internal failure.
        if e.is_redirect() {
            let mut next = std::error::Error::source(&e);
            while let Some(s) = next {
                if let Some(recovered) = Error::from_policy_display(&s.to_string()) {
                    return recovered;
                }
                next = std::error::Error::source(s);
            }
        }
        if e.is_timeout() {
            Error::timeout("client")
        } else if e.is_connect() || e.is_connection_reset() {
            Error::network("client")
        } else if e.is_redirect() || e.is_decode() {
            Error::internal("client", "request failed")
        } else {
            Error::network("client")
        }
    }
}

/// Alias for `Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_and_kind_classify() {
        let e = Error::rate_limited("qwant", Some(Duration::from_secs(30)));
        assert_eq!(e.scope(), ErrorScope::Provider);
        assert_eq!(
            e.kind(),
            &ErrorKind::RateLimited {
                retry_after: Some(Duration::from_secs(30))
            }
        );
        assert_eq!(e.http_status, Some(429));

        let e = Error::blocked("google", BlockDetails::BotDetection);
        assert_eq!(e.scope(), ErrorScope::Egress);
        assert_eq!(e.kind(), &ErrorKind::Blocked(BlockDetails::BotDetection));

        let e = Error::schema("bing", "unexpected content-type");
        assert_eq!(e.scope(), ErrorScope::Schema);

        let e = Error::invalid_query("orchestrator", "no engines");
        assert_eq!(e.scope(), ErrorScope::Query);

        let e = Error::internal("client", "build failed");
        assert_eq!(e.scope(), ErrorScope::Internal);
    }

    #[test]
    fn policy_refusals_survive_transport_boxing() {
        // The SSRF redirect guard boxes our Error through wreq; recovery
        // keys off the fixed contexts, so every refusal must round-trip.
        for context in [
            Error::SSRF_PRIVATE_IP,
            Error::SSRF_DOMAIN_POLICY,
            Error::SSRF_SCHEME,
            Error::SSRF_NO_HOST,
            Error::SSRF_DNS,
        ] {
            let original = Error::invalid_query("client", context);
            let boxed = original.to_string();
            let recovered = Error::from_policy_display(&boxed).expect("fixed context must recover");
            assert_eq!(recovered.scope(), ErrorScope::Query);
            assert_eq!(recovered.kind(), original.kind());
        }
        assert!(Error::from_policy_display("rate limited").is_none());
    }

    #[test]
    fn display_is_readable() {
        let e = Error::unavailable("mojeek", 503);
        let s = e.to_string();
        assert!(s.contains("upstream unavailable"));
        assert!(s.contains("mojeek"));
        assert!(s.contains("503"));
        let e = Error::blocked("google", BlockDetails::Cloudflare);
        assert!(e.to_string().contains("blocked (Cloudflare)"));
    }
}
