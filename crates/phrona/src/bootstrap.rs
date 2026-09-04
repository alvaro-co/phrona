//! Session bootstrap for engines that only return full results after a
//! real-browser visit (google, anna's archive; qwant accepts an
//! operator-provided cookie and never triggers this itself).
//!
//! Kept deliberately small: spawn a Chromium-family binary headless with
//! a remote-debugging port, drive it over a minimal CDP WebSocket client
//! (no external automation stack), visit one seed URL per engine, read
//! the jar via `Network.getCookies` and shut down. Seconds per engine,
//! strictly opt-in (see `SearchClient::with_auto_bootstrap`).
//!
//! Also triggered manually via `phrona bootstrap [engines...]`.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tungstenite::Message;

use crate::error::{Error, Result};

/// Per-engine bootstrap spec: seed page + the session cookie whose
/// appearance signals the visit actually completed. Note the cookie is
/// often issued seconds *after* page load, so polling is required.
pub const SEEDS: &[(&str, &str, &str)] = &[
    (
        "google",
        "https://www.google.com/search?q=test&hl=en",
        "__Secure-ENID",
    ),
    (
        "annas_archive",
        "https://annas-archive.gl/search?q=test",
        "aa_ddg_check",
    ),
    ("qwant", "https://www.qwant.com/?q=test", "datadome"),
];

/// Bound for one headless harvest attempt (spawn, navigate, clearance
/// polling, settle). The orchestrator enforces it per engine so a stuck
/// browser — or a slow first-time browser download — degrades to a normal
/// engine error instead of hanging the search. Attempts still count toward
/// the refresh spacing (see `SearchClient::search`).
pub const HARVEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Domain substrings selecting which harvested cookies belong to an engine.
fn domains_for(engine: &str) -> &'static [&'static str] {
    match engine {
        "google" => &["google.com"],
        "annas_archive" => &["annas-archive"],
        "qwant" => &["qwant.com"],
        _ => &[],
    }
}

/// Seed URL for an engine, if bootstrap applies to it.
pub fn seed_for(engine: &str) -> Option<&'static str> {
    SEEDS
        .iter()
        .find(|(name, _, _)| *name == engine)
        .map(|(_, url, _)| *url)
}

/// The clearance cookie name for an engine.
pub fn clearance_for(engine: &str) -> Option<&'static str> {
    SEEDS
        .iter()
        .find(|(name, _, _)| *name == engine)
        .map(|(_, _, c)| *c)
}

/// URL substring proving the seed page settled onto real results rather
/// than an interstitial. The generic `/search` marker never matches
/// qwant's `/?q=` results URL, so it is per-engine.
fn settle_marker(engine: &str) -> &'static str {
    match engine {
        "qwant" => "?q=",
        _ => "/search",
    }
}

/// Locate a system Chromium-family browser binary, or download the
/// official `chrome-headless-shell` build on first use when none exists.
///
/// Preference: PHRONA_BROWSER override > installed browser (zero cost) >
/// auto-download into the user cache dir (`~/.cache/phrona/browser/`).
/// The download needs no privileges and no system packages: the shell
/// build is self-contained apart from glibc/nss/fontconfig, which every
/// mainstream distro ships. Set `PHRONA_NO_DOWNLOAD=1` to disable.
pub fn find_browser() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PHRONA_BROWSER") {
        let pb = PathBuf::from(&p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    if let Some(p) = find_system_browser() {
        return Some(p);
    }
    if std::env::var_os("PHRONA_NO_DOWNLOAD").is_some() {
        return None;
    }
    match downloaded_shell() {
        Ok(p) => Some(p),
        Err(e) => {
            trace(&format!("download-failed: {e}"));
            None
        }
    }
}

fn find_system_browser() -> Option<PathBuf> {
    let names: &[&str] = match std::env::consts::OS {
        "linux" => &[
            "google-chrome-stable",
            "google-chrome",
            "brave-browser",
            "microsoft-edge-stable",
            "chromium",
            "chromium-browser",
            "thorium-browser",
            "vivaldi",
            "opera",
        ],
        "macos" => &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ],
        _ => &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        ],
    };
    for n in names {
        let candidate = if n.contains('/') || n.contains('\\') {
            PathBuf::from(n)
        } else if let Ok(which) = which(n) {
            which.into()
        } else {
            continue;
        };
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Chrome-for-testing download source for `chrome-headless-shell`.
const CFT_LATEST: &str =
    "https://googlechromelabs.github.io/chrome-for-testing/LATEST_RELEASE_STABLE";
const CFT_BASE: &str = "https://storage.googleapis.com/chrome-for-testing-public";

/// Whether a chrome-for-testing version string is plausible
/// (`"152.0.7977.64"`): 3-4 dot-separated numeric parts. The endpoint is
/// trusted, but a captive portal could return anything.
fn valid_cft_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    (3..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 6 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Upper bound for the streamed browser download; aborts a runaway
/// response instead of filling the disk.
const MAX_SHELL_ZIP: u64 = 300 * 1024 * 1024;

/// Root directory for the downloaded browser (`$PHRONA_CACHE_DIR` override).
fn browser_cache_root() -> Option<PathBuf> {
    if let Ok(x) = std::env::var("PHRONA_CACHE_DIR") {
        return Some(PathBuf::from(x));
    }
    let home = std::env::var_os("HOME")?;
    let xdg = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&home).join(".cache"));
    Some(xdg.join("phrona").join("browser"))
}

/// Drive a future to completion on a tiny current-thread runtime.
/// `harvest_blocking` is documented for blocking threads (via
/// `spawn_blocking`), but a library user on a current-thread runtime would
/// nest runtimes and panic ("Cannot start a runtime from within a
/// runtime"). When a Tokio context is already active, the work moves to a
/// fresh OS thread first, so the download path works from any caller.
fn block_on_isolated<F, T>(fut: F, what: &'static str) -> Result<T>
where
    F: std::future::Future<Output = Result<T>> + Send,
    T: Send,
{
    if tokio::runtime::Handle::try_current().is_err() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| Error::internal("bootstrap", "runtime"))?;
        return rt.block_on(fut);
    }
    std::thread::scope(|s| {
        s.spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| Error::internal("bootstrap", "runtime"))?;
            rt.block_on(fut)
        })
        .join()
        .map_err(|_| Error::internal("bootstrap", what))?
    })
}

/// A previously downloaded `chrome-headless-shell` binary, newest version
/// first. Directory names embed the version
/// (`chrome-headless-shell-<version>-linux64`); versions compare
/// numerically per dot-separated part so `99.x` never beats `152.x`.
fn find_cached_shell() -> Option<PathBuf> {
    fn version_key(dir: &std::ffi::OsStr) -> Option<(Vec<u64>, PathBuf)> {
        let name = dir.to_str()?;
        let v = name
            .strip_prefix("chrome-headless-shell-")?
            .strip_suffix("-linux64")?;
        let parts: Option<Vec<u64>> = v.split('.').map(|p| p.parse().ok()).collect();
        Some((parts?, PathBuf::from(name)))
    }
    let root = browser_cache_root()?;
    std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| {
            let dir = e.ok()?.path();
            let (key, _) = version_key(dir.file_name()?)?;
            let bin = dir
                .join("chrome-headless-shell-linux64")
                .join("chrome-headless-shell");
            bin.is_file().then_some((key, bin))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, bin)| bin)
}

/// Download (once) and return the path of the official
/// `chrome-headless-shell` binary for this platform. Linux only; other
/// platforms rely on a system browser.
fn downloaded_shell() -> Result<PathBuf> {
    if std::env::consts::OS != "linux" {
        return Err(Error::internal(
            "bootstrap",
            "auto-download only implemented for linux",
        ));
    }
    // a previous run's download wins over the network: no version fetch,
    // no failure mode when offline with a usable binary on disk
    if let Some(cached) = find_cached_shell() {
        return Ok(cached);
    }
    let version: String = block_on_isolated(
        async {
            let client = wreq::Client::builder()
                .build()
                .map_err(|_| Error::network("bootstrap"))?;
            let txt = client
                .get(CFT_LATEST)
                .send()
                .await
                .map_err(|_| Error::network(CFT_LATEST))?
                .text()
                .await
                .map_err(|_| Error::network(CFT_LATEST))?;
            Ok::<String, Error>(txt.trim().to_string())
        },
        "version fetch",
    )?;
    if !valid_cft_version(&version) {
        return Err(Error::schema("bootstrap", "bad version response"));
    }
    let root = browser_cache_root().ok_or_else(|| Error::internal("bootstrap", "no cache dir"))?;
    let dir = root.join(format!("chrome-headless-shell-{version}-linux64"));
    let bin = dir
        .join("chrome-headless-shell-linux64")
        .join("chrome-headless-shell");
    if bin.is_file() {
        return Ok(bin);
    }
    std::fs::create_dir_all(&dir).map_err(|_| Error::internal("bootstrap", "cache dir create"))?;
    let url = format!("{CFT_BASE}/{version}/linux64/chrome-headless-shell-linux64.zip");
    trace(&format!(
        "downloading chrome-headless-shell {version} (~95MB)"
    ));
    let zip_path = dir.join("shell.zip");
    // stream to disk: the ~95MB zip must not sit in RAM twice (response
    // buffer plus the copy handed to the seed-file write)
    {
        use futures::StreamExt;
        let owned_url = url.clone();
        let mut file = std::fs::File::create(&zip_path)
            .map_err(|_| Error::internal("bootstrap", "zip create"))?;
        let mut written: u64 = 0;
        block_on_isolated(
            async move {
                let client = wreq::Client::builder()
                    .build()
                    .map_err(|_| Error::network("bootstrap"))?;
                let resp = client
                    .get(owned_url)
                    .send()
                    .await
                    .map_err(|_| Error::network("cft-download"))?;
                if !resp.status().is_success() {
                    return Err(Error::unavailable("cft-download", resp.status().as_u16()));
                }
                let mut stream = resp.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| Error::network("cft-download"))?;
                    written += chunk.len() as u64;
                    if written > MAX_SHELL_ZIP {
                        return Err(Error::schema("bootstrap", "download too large"));
                    }
                    file.write_all(&chunk)
                        .map_err(|_| Error::internal("bootstrap", "zip write"))?;
                }
                Ok::<(), Error>(())
            },
            "browser download",
        )?;
    }
    {
        let file =
            std::fs::File::open(&zip_path).map_err(|_| Error::internal("bootstrap", "zip open"))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|_| Error::schema("bootstrap", "bad zip"))?;
        archive
            .extract(&dir)
            .map_err(|_| Error::internal("bootstrap", "zip extract"))?;
    }
    let _ = std::fs::remove_file(&zip_path);
    // executable bit (unix only; windows derives it from the extension)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .map_err(|_| Error::internal("bootstrap", "chmod"))?;
    }
    trace("download-complete");
    Ok(bin)
}

/// Minimal `which`: scan PATH for an executable name.
fn which(name: &str) -> std::result::Result<String, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        // a non-executable file shadowing a real binary must not win:
        // the spawn would fail later with a confusing error
        if p.is_file() && is_executable(&p) {
            return Ok(p.to_string_lossy().into_owned());
        }
    }
    Err(())
}

#[cfg(unix)]
fn is_executable(p: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(_p: &std::path::Path) -> bool {
    true
}

fn trace(stage: &str) {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    if std::env::var_os("PHRONA_BOOTSTRAP_DEBUG").is_some() {
        let t = START.get_or_init(Instant::now);
        eprintln!("[bootstrap:{stage} +{:.2}s]", t.elapsed().as_secs_f64());
    }
}

// ---------------------------------------------------------------------------
// local cookie cache (optional, next to phrona.yaml)
// ---------------------------------------------------------------------------

/// Cache file name; resolved next to the active `phrona.yaml` (or cwd),
/// overridable via `PHRONA_COOKIE_CACHE`.
pub fn cache_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PHRONA_COOKIE_CACHE") {
        return Some(PathBuf::from(p));
    }
    let dir = match std::env::var("PHRONA_CONFIG_PATH") {
        // mirror `PhronaConfig::load`: an empty path means "unset"
        Ok(p) if !p.is_empty() => PathBuf::from(p).parent().map(|d| d.to_path_buf()),
        Ok(_) => std::env::current_dir().ok(),
        Err(_) => std::env::current_dir().ok(),
    };
    dir.map(|d| d.join("phrona.cookies.json"))
}

/// Load a previously stored cookie header for `engine` from the local
/// cache. Returns `(header, updated_at_unix_secs)`.
pub fn load_cached(engine: &str) -> Option<(String, u64)> {
    let path = cache_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let e = v.get(engine)?;
    let header = e.get("cookies")?.as_str()?.to_string();
    let at = e.get("updated_at").and_then(|t| t.as_u64())?;
    if header.is_empty() {
        return None;
    }
    Some((header, at))
}

/// Serializes concurrent `store_cached` calls in-process; the final
/// `rename` is atomic, so readers never see a torn file.
static CACHE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Store a cookie header for `engine` in the local cache (best-effort).
pub fn store_cached(engine: &str, header: &str) {
    let Some(path) = cache_path() else { return };
    let _guard = CACHE_LOCK.lock();
    let mut root: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    root[engine] = serde_json::json!({
        "cookies": header,
        "updated_at": now,
    });
    if let Ok(txt) = serde_json::to_string_pretty(&root) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, txt).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Suggested minimum re-harvest spacing per engine: google's ENID lives
/// long, anna's-archive clearance is short-lived.
pub fn min_refresh_interval(engine: &str) -> Duration {
    match engine {
        "google" | "qwant" => Duration::from_secs(3 * 3600),
        _ => crate::engine::MIN_REHARVEST_INTERVAL,
    }
}

/// UA presented by harvested sessions. Headless builds advertise their
/// own variant here, which some upstreams treat differently; presenting
/// a standard desktop identity keeps the session consistent with the
/// client that will reuse its cookies.
const SESSION_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";

struct BrowserProc {
    child: Child,
    profile_dir: PathBuf,
}

impl Drop for BrowserProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // best-effort profile cleanup; ignore errors
        let _ = std::fs::remove_dir_all(&self.profile_dir);
    }
}

fn spawn_browser(engine: &str) -> Result<(BrowserProc, u16)> {
    let binary = find_browser().ok_or_else(|| {
        Error::internal(
            "bootstrap",
            "no browser available and auto-download disabled/failed",
        )
    })?;
    // bind port 0 trick to get a free port
    let port = {
        let s = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|_| Error::internal("bootstrap", "port bind failed"))?;
        s.local_addr()
            .map_err(|_| Error::internal("bootstrap", "addr failed"))?
            .port()
    };
    let profile_dir =
        std::env::temp_dir().join(format!("phrona-bootstrap-{engine}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&profile_dir);
    std::fs::create_dir_all(&profile_dir)
        .map_err(|_| Error::internal("bootstrap", "profile dir failed"))?;
    let mut cmd = Command::new(&binary);
    cmd.arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--window-size=1280,800",
            // headless-shell creates no page target until given a URL
            "about:blank",
        ]);
    cmd.arg("--headless=new").arg("--disable-gpu");
    let bin_display = binary.display().to_string();
    let child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            // `context` is static: keep the detail in the debug trace
            // instead of leaking a heap string per failure.
            trace(&format!("browser spawn failed ({e}): {bin_display}"));
            Error::internal("bootstrap", "browser spawn failed")
        })?;
    Ok((BrowserProc { child, profile_dir }, port))
}

/// Wait until /json/version answers; returns when CDP is ready.
fn wait_cdp(port: u16, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_millis(2000)) {
            let mut s = s;
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
            let req = format!(
                "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            );
            if s.write_all(req.as_bytes()).is_ok() {
                let mut buf = String::new();
                // bounded: 2s socket timeout or EOF ends this
                let _ = s.read_to_string(&mut buf);
                if buf.contains("200 OK") || buf.contains("webSocketDebuggerUrl") {
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err(Error::timeout("bootstrap"))
}

/// Bounded TCP connect to the local CDP endpoint.
fn tcp_connect(port: u16) -> Result<TcpStream> {
    let s = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}")
            .parse()
            .map_err(|_| Error::network("bootstrap"))?,
        Duration::from_secs(3),
    )
    .map_err(|_| Error::network("bootstrap"))?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    s.set_write_timeout(Some(Duration::from_secs(3))).ok();
    Ok(s)
}

/// Connect to a page-level CDP WebSocket using tungstenite\'s own
/// handshake (manual handshakes desync the frame stream).
fn ws_connect(host: &str, port: u16, ws_path: &str) -> Result<tungstenite::WebSocket<TcpStream>> {
    use tungstenite::client::IntoClientRequest;
    let addr = format!("127.0.0.1:{port}")
        .parse()
        .map_err(|_| Error::network("bootstrap"))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
        .map_err(|_| Error::network("bootstrap"))?;
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(Duration::from_millis(1200)))
        .ok();

    let uri = format!("ws://{host}:{port}{ws_path}");
    let request = uri
        .into_client_request()
        .map_err(|_| Error::network("bootstrap"))?;
    tungstenite::client(request, stream)
        .map(|(ws, _)| ws)
        .map_err(|_| Error::network("bootstrap"))
}

fn ws_send(ws: &mut tungstenite::WebSocket<TcpStream>, v: &serde_json::Value) -> Result<()> {
    if std::env::var_os("PHRONA_BOOTSTRAP_DEBUG").is_some() {
        eprintln!("[bootstrap:cdp-send] {:.140}", v.to_string());
    }
    ws.send(Message::Text(v.to_string()))
        .map_err(|_| Error::network("bootstrap"))
}

/// Read next CDP message; ignores events, returns params JSON for matching id.
fn ws_recv_until(
    ws: &mut tungstenite::WebSocket<TcpStream>,
    want_id: u64,
    timeout: Duration,
) -> Result<serde_json::Value> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err(Error::timeout("bootstrap"));
        }
        // short socket timeouts let the deadline bite; idle reads just loop
        let msg = match read_message(ws) {
            Ok(m) => m,
            Err(Error {
                kind: crate::error::ErrorKind::Timeout,
                ..
            }) => continue,
            Err(e) => return Err(e),
        };
        if let Message::Text(txt) = msg {
            if std::env::var_os("PHRONA_BOOTSTRAP_DEBUG").is_some() {
                eprintln!("[bootstrap:cdp-recv] {:.120}", txt);
            }
            let v: serde_json::Value = serde_json::from_str(&txt)
                .map_err(|_| Error::schema("bootstrap", "bad cdp json"))?;
            if v.get("id").and_then(|i| i.as_u64()) == Some(want_id) {
                if let Some(err) = v.get("error") {
                    trace(&format!("cdp error response: {err}"));
                    return Err(Error::schema("bootstrap", "cdp error response"));
                }
                return Ok(v.get("result").cloned().unwrap_or(serde_json::Value::Null));
            }
        } else if let Message::Ping(payload) = msg {
            // unanswered pings let the browser half-close an idle session
            let _ = ws.send(Message::Pong(payload));
        }
    }
}

fn read_message(ws: &mut tungstenite::WebSocket<TcpStream>) -> Result<Message> {
    // tungstenite write/read need &mut; just delegate
    ws.read().map_err(|e| match e {
        tungstenite::Error::Io(io) if io.kind() == std::io::ErrorKind::WouldBlock => {
            Error::timeout("bootstrap")
        }
        _ => Error::network("bootstrap"),
    })
}

type Ws = tungstenite::WebSocket<TcpStream>;

/// Poll `/json/list` until a page target exists; returns its ws URL.
fn resolve_page_ws(port: u16) -> Result<String> {
    for _ in 0..10 {
        let mut s = tcp_connect(port)?;
        s.write_all(
            format!(
                "GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .map_err(|_| Error::network("bootstrap"))?;
        let mut body = String::new();
        {
            let mut r = BufReader::new(s.try_clone().map_err(|_| Error::network("bootstrap"))?);
            // bounded: the 3s socket read timeout ends this read
            let _ = r.read_to_string(&mut body);
        }
        if let Ok(targets) = serde_json::from_str::<serde_json::Value>(
            body[body.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0)..].trim(),
        ) {
            if let Some(u) = targets
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
                })
                .and_then(|t| t.get("webSocketDebuggerUrl"))
                .and_then(|v| v.as_str())
            {
                return Ok(u.to_string());
            }
        }
        std::thread::sleep(Duration::from_millis(700));
    }
    Err(Error::schema("bootstrap", "no page target"))
}

/// Session-consistency patches applied before navigation (identity,
/// client hints, automation flags). Some upstreams issue different
/// treatment to unpatched headless sessions.
fn apply_session_identity(ws: &mut Ws, next_id: &mut u64) -> Result<()> {
    ws_send(
        ws,
        &serde_json::json!({"id": *next_id, "method": "Network.enable"}),
    )?;
    let _ = ws_recv_until(ws, *next_id, Duration::from_secs(5));
    *next_id += 1;
    ws_send(
        ws,
        &serde_json::json!({"id": *next_id, "method": "Network.setUserAgentOverride",
            "params": {"userAgent": SESSION_UA,
                "userAgentMetadata": {"brands": [
                    {"brand": "Chromium", "version": "148"},
                    {"brand": "Google Chrome", "version": "148"}],
                    "fullVersionList": [{"brand": "Chromium", "version": "148"},
                        {"brand": "Google Chrome", "version": "148"}],
                    "fullVersion": "148.0.0.0", "platform": "Linux",
                    "platformVersion": "6.8.0", "architecture": "x86",
                    "model": "", "mobile": false,
                    "bitness": "64", "wow64": false}}}),
    )?;
    let _ = ws_recv_until(ws, *next_id, Duration::from_secs(5));
    *next_id += 1;
    ws_send(
        ws,
        &serde_json::json!({"id": *next_id, "method": "Page.addScriptToEvaluateOnNewDocument",
            "params": {"source":
                "Object.defineProperty(navigator,'webdriver',{get:()=>undefined});"}}),
    )?;
    let _ = ws_recv_until(ws, *next_id, Duration::from_secs(5));
    *next_id += 1;
    Ok(())
}

/// Harvest session cookies for `engine` by driving the browser headless
/// through its seed page. Returns a `Cookie` header value.
///
/// Blocking (spawns a process and waits); call from a tokio context via
/// `tokio::task::spawn_blocking`.
pub fn harvest_blocking(engine: &str) -> Result<String> {
    let Some(seed) = seed_for(engine) else {
        return Err(Error::invalid_query(
            "bootstrap",
            "engine has no bootstrap seed",
        ));
    };
    trace("spawn");
    let (_proc_guard, port) = spawn_browser(engine)?;
    wait_cdp(port, Duration::from_secs(20))?;
    trace("waitcdp");
    trace("targetlist");
    let ws_url = resolve_page_ws(port)?;

    // parse ws url
    let rest = ws_url
        .strip_prefix("ws://")
        .ok_or_else(|| Error::schema("bootstrap", "unexpected ws scheme"))?;
    let (hostport, path) = rest
        .split_once('/')
        .ok_or_else(|| Error::schema("bootstrap", "bad ws url"))?;
    let (host, port_s) = hostport
        .split_once(':')
        .ok_or_else(|| Error::schema("bootstrap", "bad ws authority"))?;
    let ws_port: u16 = port_s
        .parse()
        .map_err(|_| Error::schema("bootstrap", "bad ws port"))?;
    trace("wsconnect");
    let mut ws = ws_connect(host, ws_port, &format!("/{path}"))?;
    let mut next_id: u64 = 1;

    // events need the Page domain enabled
    let _ = ws_send(
        &mut ws,
        &serde_json::json!({"id": next_id, "method": "Page.enable"}),
    );
    let _ = ws_recv_until(&mut ws, next_id, Duration::from_secs(10));
    next_id += 1;
    // present a consistent session identity before any navigation
    apply_session_identity(&mut ws, &mut next_id)?;
    trace("identity-applied");
    // navigate
    ws_send(
        &mut ws,
        &serde_json::json!({
            "id": next_id, "method": "Page.navigate", "params": {"url": seed}
        }),
    )?;
    trace("navigate-sent");
    let nav_ack = next_id;
    next_id += 1;
    ws_recv_until(&mut ws, nav_ack, Duration::from_secs(20))?;
    trace("navigate-ack");
    // wait for Page.loadEventFired (evaluate() queues while the renderer
    // is busy, so polling readyState right after navigate() starves)
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        if Instant::now() >= deadline {
            break; // proceed anyway; cookies may already be set
        }
        match read_message(&mut ws) {
            Ok(Message::Text(txt)) => {
                if std::env::var_os("PHRONA_BOOTSTRAP_DEBUG").is_some() {
                    eprintln!("[bootstrap:cdp-evt] {:.100}", txt);
                }
                let v: serde_json::Value =
                    serde_json::from_str(&txt).unwrap_or(serde_json::Value::Null);
                if v.get("method").and_then(|m| m.as_str()) == Some("Page.loadEventFired") {
                    trace("load-fired");
                    break;
                }
            }
            Ok(Message::Ping(p)) => {
                let _ = ws.send(Message::Pong(p));
            }
            Ok(_) => {}
            Err(Error {
                kind: crate::error::ErrorKind::Timeout,
                ..
            }) => {
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(e),
        }
    }

    // The upstream session often completes seconds after load; poll the
    // jar until the engine's marker cookie appears.
    let clearance = clearance_for(engine);
    let deadline = Instant::now() + Duration::from_secs(40);
    let mut cookies;
    loop {
        ws_send(
            &mut ws,
            &serde_json::json!({"id": next_id, "method": "Network.getCookies"}),
        )?;
        let res = ws_recv_until(&mut ws, next_id, Duration::from_secs(10))?;
        next_id += 1;
        cookies = res
            .get("cookies")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let got_clearance = clearance
            .map(|c| {
                cookies
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .any(|x| x.get("name").and_then(|n| n.as_str()) == Some(c))
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(true);
        if got_clearance || Instant::now() >= deadline {
            trace(if got_clearance {
                "clearance-ok"
            } else {
                "clearance-timeout"
            });
            // No marker cookie means the visit did not complete - the jar
            // would be unusable. Fail so callers don't cache or use it.
            if !got_clearance {
                return Err(Error::blocked(
                    "bootstrap",
                    crate::error::BlockDetails::BotDetection,
                ));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(1500));
    }

    // Settle: many sites reload onto the real page once their background
    // handshake finishes, issuing final (rotated) cookies afterwards.
    // Wait for that navigation to complete before reading the jar.
    let settle_deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < settle_deadline {
        ws_send(
            &mut ws,
            &serde_json::json!({
                "id": next_id, "method": "Runtime.evaluate",
                "params": {"expression":
                    "(location.href+'|'+document.readyState)"}
            }),
        )?;
        match ws_recv_until(&mut ws, next_id, Duration::from_secs(8)) {
            Ok(res) => {
                next_id += 1;
                let state = res
                    .pointer("/result/value")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                trace("settle-probe");
                let (href, ready) = state.split_once('|').unwrap_or(("", ""));
                let on_target = href.contains(settle_marker(engine))
                    && !href.contains("check=1")
                    && !href.contains("/check");
                if on_target && ready == "complete" {
                    trace("settled-on-target");
                    break;
                }
            }
            Err(Error {
                kind: crate::error::ErrorKind::Timeout,
                ..
            }) => {
                next_id += 1;
                continue;
            }
            Err(e) => return Err(e),
        }
        std::thread::sleep(Duration::from_millis(700));
    }

    // Re-read the jar AFTER settling: rotation happens during/after the
    // reload, so pre-settle cookies may already be superseded.
    ws_send(
        &mut ws,
        &serde_json::json!({"id": next_id, "method": "Network.getCookies"}),
    )?;
    if let Ok(res) = ws_recv_until(&mut ws, next_id, Duration::from_secs(10)) {
        if let Some(arr) = res.get("cookies") {
            cookies = arr.clone();
        }
    }

    let matchers = domains_for(engine);
    let mut pairs: Vec<String> = Vec::new();
    if let Some(arr) = cookies.as_array() {
        for c in arr {
            let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
            let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let value = c.get("value").and_then(|v| v.as_str()).unwrap_or("");
            if matchers.iter().any(|m| domain.contains(m)) && !name.is_empty() {
                pairs.push(format!("{name}={value}"));
            }
        }
    }
    if pairs.is_empty() {
        return Err(Error::blocked(
            "bootstrap",
            crate::error::BlockDetails::BotDetection,
        ));
    }
    Ok(pairs.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_and_domains_are_wired() {
        assert_eq!(
            seed_for("google"),
            Some("https://www.google.com/search?q=test&hl=en")
        );
        assert!(
            seed_for("mojeek").is_none(),
            "native engines must not be listed"
        );
        assert_eq!(domains_for("annas_archive"), &["annas-archive"]);
        assert_eq!(domains_for("qwant"), &["qwant.com"]);
    }

    #[test]
    fn settle_markers_match_their_seeds() {
        // every seed URL must contain its own settle marker, or harvests
        // loop until the 45s settle timeout on every run
        for (engine, seed, _) in SEEDS {
            assert!(
                seed.contains(settle_marker(engine)),
                "{engine} seed {seed} lacks its settle marker"
            );
        }
    }

    #[test]
    fn version_validation_rejects_garbage() {
        assert!(valid_cft_version("152.0.7977.64"));
        assert!(valid_cft_version("100.0.1"));
        assert!(!valid_cft_version(""));
        assert!(!valid_cft_version("stable"));
        assert!(!valid_cft_version("<html>captive portal</html>"));
        assert!(!valid_cft_version("152.0.7977.64\nmalicious"));
    }

    #[test]
    fn finds_some_browser_or_none_gracefully() {
        // on dev machines there is usually one; on CI none - both are fine,
        // the point is this never panics
        let _ = find_browser();
    }

    #[test]
    fn cached_shell_prefers_newest_numeric_version() {
        // lexicographic order would rank "99.x" above "152.x"; versions
        // must compare numerically per part
        let root = std::env::temp_dir().join(format!("phrona-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for v in ["99.0.1", "152.0.7977.64"] {
            let bin = root
                .join(format!("chrome-headless-shell-{v}-linux64"))
                .join("chrome-headless-shell-linux64")
                .join("chrome-headless-shell");
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(&bin, b"x").unwrap();
        }
        unsafe {
            std::env::set_var("PHRONA_CACHE_DIR", &root);
        }
        let found = find_cached_shell().expect("cached binary must be found");
        unsafe {
            std::env::remove_var("PHRONA_CACHE_DIR");
        }
        assert!(
            found.to_string_lossy().contains("152.0.7977.64"),
            "got {found:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
        unsafe {
            std::env::set_var("PHRONA_CACHE_DIR", &root);
        }
        assert!(find_cached_shell().is_none(), "empty cache finds nothing");
        unsafe {
            std::env::remove_var("PHRONA_CACHE_DIR");
        }
    }

    #[tokio::test]
    async fn block_on_isolated_works_inside_runtime() {
        // inside a runtime context the work must hop threads instead of
        // nesting runtimes (which panics)
        let v =
            block_on_isolated(async { Ok::<i32, Error>(41 + 1) }, "test").expect("isolated run");
        assert_eq!(v, 42);
    }
}
