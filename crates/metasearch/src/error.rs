use std::fmt;

#[derive(Debug, Clone)]
pub enum Error {
    Http(String),
    Request(String),
    Parse(String),
    Engine(String),
    RateLimited(String),
    Timeout(String),
    NoResults(String),
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Http(m) => write!(f, "http error: {m}"),
            Error::Request(m) => write!(f, "request error: {m}"),
            Error::Parse(m) => write!(f, "parse error: {m}"),
            Error::Engine(m) => write!(f, "engine error: {m}"),
            Error::RateLimited(m) => write!(f, "rate limited: {m}"),
            Error::Timeout(m) => write!(f, "timeout: {m}"),
            Error::NoResults(m) => write!(f, "no results: {m}"),
            Error::Io(m) => write!(f, "io error: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
