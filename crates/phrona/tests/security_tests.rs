//! Security hardening tests: SSRF range rejection (incl. DNS-rebinding
//! vectors), redirect-loop validation, and XSS-oriented URL filtering in
//! extracted pages.

use std::net::IpAddr;

use phrona::extract_from_html;
use phrona::{HttpClient, extract, is_safe_ip};

fn expect_rejected(ip: &str) {
    let ip: IpAddr = ip.parse().unwrap();
    assert!(!is_safe_ip(ip), "{ip} must be rejected");
}

fn expect_allowed(ip: &str) {
    let ip: IpAddr = ip.parse().unwrap();
    assert!(is_safe_ip(ip), "{ip} must be allowed");
}

#[test]
fn rejects_all_private_ipv4_ranges() {
    for ip in [
        "0.0.0.0",         // 0.0.0.0/8
        "0.1.2.3",         // 0.0.0.0/8
        "10.0.0.1",        // 10.0.0.0/8
        "10.255.255.254",  // 10.0.0.0/8
        "100.64.0.1",      // 100.64.0.0/10 CGNAT
        "100.127.255.254", // 100.64.0.0/10 CGNAT
        "127.0.0.1",       // 127.0.0.0/8
        "127.8.8.8",       // 127.0.0.0/8
        "169.254.169.254", // 169.254.0.0/16 (AWS/GCP metadata)
        "169.254.0.1",     // 169.254.0.0/16
        "172.16.0.1",      // 172.16.0.0/12
        "172.31.255.254",  // 172.16.0.0/12
        "192.0.0.1",       // 192.0.0.0/24
        "192.0.2.1",       // 192.0.2.0/24 (documentation)
        "192.88.99.1",     // 192.88.99.0/24 (6to4 relay)
        "192.168.1.1",     // 192.168.0.0/16
        "198.18.0.1",      // 198.18.0.0/15 (benchmark)
        "198.19.255.254",  // 198.18.0.0/15
        "198.51.100.1",    // 198.51.100.0/24 (documentation)
        "203.0.113.1",     // 203.0.113.0/24 (documentation)
        "224.0.0.1",       // 224.0.0.0/4 (multicast)
        "239.255.255.255", // 224.0.0.0/4
        "240.0.0.1",       // 240.0.0.0/4 (reserved)
        "255.255.255.255", // broadcast
    ] {
        expect_rejected(ip);
    }
}

#[test]
fn rejects_all_private_ipv6_ranges() {
    for ip in [
        "::1",               // ::1/128 loopback
        "::",                // ::/128 unspecified
        "::ffff:127.0.0.1",  // ::ffff:0:0/96 mapped loopback
        "::ffff:10.0.0.1",   // ::ffff:0:0/96 mapped RFC1918
        "::ffff:8.8.8.8",    // ::ffff:0:0/96 mapped public (whole range rejected)
        "::127.0.0.1",       // IPv4-compatible loopback
        "64:ff9b:1::1",      // 64:ff9b:1::/48 NAT64
        "100::1",            // 100::/64 discard
        "2001:db8::1",       // 2001:db8::/32 documentation
        "2002::1",           // 2002::/16 6to4
        "fc00::1",           // fc00::/7 ULA
        "fd12:3456:789a::1", // fc00::/7 ULA
        "fe80::1",           // fe80::/10 link-local
        "ff02::1",           // multicast
    ] {
        expect_rejected(ip);
    }
}

#[test]
fn allows_public_addresses() {
    for ip in [
        "8.8.8.8",
        "1.1.1.1",
        "93.184.216.34",
        "172.32.0.1",  // outside 172.16/12
        "169.255.0.1", // outside 169.254/16
        "192.1.1.1",   // outside 192.0.0/24 and 192.0.2/24
        "2001:4860:4860::8888",
        "2606:4700:4700::1111",
        "2001:db9::1", // outside 2001:db8::/32
    ] {
        expect_allowed(ip);
    }
}

/// DNS-rebinding vectors: a name that the resolver maps to a private IP must
/// be blocked before any request is sent. `localhost` is the canonical
/// rebinding target via /etc/hosts; IP literals cover rebinding via poisoned
/// DNS answers (the resolved address itself is what gets validated).
#[tokio::test]
async fn extract_blocks_dns_rebinding_vectors() {
    let client = HttpClient::builder().build().unwrap();
    for url in [
        "http://localhost/",
        "http://localhost:8080/",
        "http://127.0.0.1/",
        "http://127.0.0.1:8080/",
        "http://169.254.169.254/latest/meta-data/",
        "http://10.0.0.1/",
        "http://192.168.1.1/",
        "http://[::1]/",
        "http://[::ffff:127.0.0.1]/",
        "http://[fc00::1]/",
    ] {
        let err = extract(&client, url, 100, None).await.unwrap_err();
        assert!(err.to_string().contains("SSRF blocked"), "{url}: got {err}");
    }
}

#[tokio::test]
async fn extract_blocks_non_http_schemes() {
    let client = HttpClient::builder().build().unwrap();
    for url in [
        "javascript:alert(1)",
        "javascript:document.location='http://evil.example/'",
        "data:text/html,<script>alert(1)</script>",
        "vbscript:msgbox(1)",
        "file:///etc/passwd",
        "ftp://example.com/file",
        "",
        "not a url",
    ] {
        let err = extract(&client, url, 100, None).await.unwrap_err();
        assert!(
            err.to_string().contains("invalid query"),
            "{url:?}: got {err}"
        );
    }
}

/// XSS-string injections in HTML: only http(s) image URLs survive extraction;
/// javascript:/data:/vbscript: sources are dropped, and attacker HTML inside
/// text fields is returned as plain text (the frontend escapes it before
/// injection into the DOM).
#[test]
fn extract_from_html_filters_xss_vectors() {
    let html = r#"
<!doctype html><html><head><title><img src=x onerror=alert(1)></title>
<meta name="description" content="<script>alert(1)</script>">
</head><body><main>
<h1>Hello</h1>
<p>Trusted content.</p>
<img src="https://example.com/ok.png">
<img src="javascript:alert(1)">
<img src="data:text/html,<svg onload=alert(1)>">
<img src="vbscript:msgbox(1)">
<img src="//cdn.example.com/proto-relative.png">
<img src="/relative.png">
</main></body></html>"#;
    let page = extract_from_html(html, "https://example.com/page", 500, None);
    assert_eq!(page.images, ["https://example.com/ok.png"]);
    assert!(!page.images.iter().any(|u| !u.starts_with("https://")));
    assert_eq!(page.title, "<img src=x onerror=alert(1)>");
    assert_eq!(page.description, "<script>alert(1)</script>");
    assert!(page.text.contains("Trusted content."));
}
