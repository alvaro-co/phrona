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
    // sync context: run the async fetch on a tiny current-thread runtime
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Error::internal("bootstrap", "runtime"))?;
    let version: String = rt.block_on(async {
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
    })?;
    if version.is_empty() || !version.contains('.') {
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
    let bytes: Vec<u8> = {
        let owned_url = url.clone();
        rt.block_on(async move {
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
            let buf = resp
                .bytes()
                .await
                .map_err(|_| Error::network("cft-download"))?;
            Ok::<Vec<u8>, Error>(buf.to_vec())
        })?
    };
    let zip_path = dir.join("shell.zip");
    std::fs::write(&zip_path, &bytes).map_err(|_| Error::internal("bootstrap", "zip write"))?;
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
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
        .map_err(|_| Error::internal("bootstrap", "chmod"))?;
    trace("download-complete");
    Ok(bin)
}

/// Minimal `which`: scan PATH for an executable name.
#[allow(dead_code)]
fn which(name: &str) -> std::result::Result<String, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Ok(p.to_string_lossy().into_owned());
        }
    }
    Err(())
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
        Ok(p) => PathBuf::from(p).parent().map(|d| d.to_path_buf()),
        Err(_) => std::fs::canonicalize(".")
            .ok()
            .map(|_| std::path::PathBuf::from(".")),
    };
    dir.map(|d| d.join("phrona.cookies.json"))
}

fn cache_entry_key(engine: &str) -> String {
    engine.to_string()
}

/// Load a previously stored cookie header for `engine` from the local
/// cache. Returns `(header, updated_at_unix_secs)`.
pub fn load_cached(engine: &str) -> Option<(String, u64)> {
    let path = cache_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let e = v.get(cache_entry_key(engine))?;
    let header = e.get("cookies")?.as_str()?.to_string();
    let at = e.get("updated_at").and_then(|t| t.as_u64())?;
    if header.is_empty() {
        return None;
    }
    Some((header, at))
}

/// Store a cookie header for `engine` in the local cache (best-effort).
pub fn store_cached(engine: &str, header: &str) {
    let Some(path) = cache_path() else { return };
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
    root[cache_entry_key(engine)] = serde_json::json!({
        "cookies": header,
        "updated_at": now,
    });
    if let Ok(txt) = serde_json::to_string_pretty(&root) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, txt).is_ok() {
            let _ = std::fs::rename(&tmp, &path); // atomic-enough locally
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
    #[allow(dead_code)]
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
            Error::internal(
                "bootstrap",
                Box::leak(format!("browser spawn failed ({e}): {bin_display}").into_boxed_str()),
            )
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
                    return Err(Error::schema(
                        "bootstrap",
                        Box::leak(err.to_string().into_boxed_str()),
                    ));
                }
                return Ok(v.get("result").cloned().unwrap_or(serde_json::Value::Null));
            }
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
                let on_target = href.contains("/search")
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
    fn finds_some_browser_or_none_gracefully() {
        // on dev machines there is usually one; on CI none - both are fine,
        // the point is this never panics
        let _ = find_browser();
    }
}
