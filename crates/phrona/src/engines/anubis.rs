//! Minimal solver for [Anubis](https://anubis.techaro.lol) proof-of-work
//! interstitials.
//!
//! Some upstream engines (currently Startpage) sit behind Anubis, a
//! hashcash-style challenge that a real browser solves in JavaScript before
//! being granted a clearance cookie. The protocol is deterministic and cheap
//! at the difficulties engines serve, so it is implemented natively:
//!
//! 1. The interstitial page embeds `<script id="anubis_challenge"
//!    type="application/json">{...}</script>` carrying `rules.difficulty`
//!    (leading zero bits required) and `challenge.randomData` / `challenge.id`.
//! 2. Find the smallest nonce >= 0 such that
//!    `SHA-256(random_data ++ decimal_nonce)` has at least `difficulty`
//!    leading zero bits (exactly what the upstream worker checks: whole zero
//!    bytes first, then a partial byte for non-multiple-of-8 difficulties).
//! 3. GET `<origin>/.within.website/x/cmd/anubis/api/pass-challenge` with
//!    `id`, `response` (hex digest), `nonce`, `redir` and `elapsedTime`;
//!    the response sets the clearance cookie in the client's cookie jar.
//! 4. Re-issue the original request on the same client.

use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::client::HttpClient;
use crate::error::{BlockDetails, Error, Result};
use crate::parse;

/// The `<script id>` that carries the challenge payload.
const CHALLENGE_MARKER: &str = "anubis_challenge";

/// Hard nonce ceiling: difficulty 4 needs ~65k hashes on average; even a
/// pathological difficulty 8 stays far below this on any modern CPU.
/// Anything past it is an upstream policy change or a deliberate DoS - give
/// up and report a block instead of burning CPU forever.
const MAX_NONCE: u64 = 1 << 28;

/// Difficulties above this need 16^d expected hashes (difficulty 9 ≈ 68B);
/// refusing them keeps a hostile policy change from pinning a CPU.
const MAX_SOLVABLE_DIFFICULTY: u32 = 8;

/// A parsed Anubis challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    /// Challenge id (`challenge.id`).
    pub id: String,
    /// Salt hashed together with the nonce (`challenge.randomData`).
    pub random_data: String,
    /// Required leading zero bits (`rules.difficulty`).
    pub difficulty: u32,
}

impl Challenge {
    /// Whether a response body is an Anubis interstitial (fast check).
    pub fn present_in(html: &str) -> bool {
        html.contains(CHALLENGE_MARKER)
    }

    /// Extract the challenge from an Anubis interstitial page. `None` when
    /// the page carries no parseable challenge.
    pub fn extract(html: &str) -> Option<Self> {
        let idx = html.find(CHALLENGE_MARKER)?;
        // The JSON blob is the text of the script tag following the marker:
        // `<script id="anubis_challenge" type="application/json">{...}</script>`
        let after = &html[idx + CHALLENGE_MARKER.len()..];
        let start = after.find('>')? + 1;
        let end = after[start..].find("</script>")? + start;
        let json: serde_json::Value = serde_json::from_str(after[start..end].trim()).ok()?;
        let random_data = json
            .pointer("/challenge/randomData")
            .and_then(|v| v.as_str())
            .map(str::to_owned)?;
        if random_data.is_empty() {
            return None;
        }
        // `rules.difficulty` is authoritative; some builds also repeat it
        // under `challenge.difficulty`, so accept either.
        let difficulty = json
            .pointer("/rules/difficulty")
            .or_else(|| json.pointer("/challenge/difficulty"))
            .and_then(|v| v.as_u64())?;
        if difficulty == 0 || difficulty > 16 {
            return None;
        }
        let id = json
            .pointer("/challenge/id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        Some(Self {
            id,
            random_data,
            difficulty: difficulty as u32,
        })
    }

    /// Solve the proof-of-work: returns `(hex_digest, nonce)`, or `None`
    /// when no solution exists below `MAX_NONCE`.
    ///
    /// Upstream semantics (Anubis worker `sha256-*.mjs`): `difficulty`
    /// counts leading zero *hex digits* - i.e. `floor(difficulty / 2)`
    /// whole zero bytes plus a high-nibble zero when `difficulty` is odd.
    ///
    /// Difficulties past `MAX_SOLVABLE_DIFFICULTY` are refused outright:
    /// at 16^d expected hashes they are a deliberate CPU-DoS, not a real
    /// challenge (upstream ships difficulty 4).
    pub fn solve(&self) -> Option<(String, u64)> {
        if self.difficulty > MAX_SOLVABLE_DIFFICULTY {
            return None;
        }
        let diff = self.difficulty;
        let full_bytes = (diff / 2) as usize;
        let odd_nibble = diff % 2 == 1;
        for nonce in 0..MAX_NONCE {
            let mut hasher = Sha256::new();
            hasher.update(self.random_data.as_bytes());
            hasher.update(nonce.to_string().as_bytes());
            let digest = hasher.finalize();
            if digest[..full_bytes].iter().any(|&b| b != 0) {
                continue;
            }
            if odd_nibble && (digest[full_bytes] >> 4) != 0 {
                continue;
            }
            let mut hex = String::with_capacity(digest.len() * 2);
            for b in digest.iter() {
                use std::fmt::Write as _;
                let _ = write!(hex, "{b:02x}");
            }
            return Some((hex, nonce));
        }
        None
    }

    /// Present a solved proof to the `pass-challenge` endpoint on `origin`
    /// (e.g. `https://www.startpage.com`). On success the clearance cookie
    /// lands in `client`'s cookie jar and subsequent requests pass.
    pub async fn redeem(&self, client: &HttpClient, origin: &str, redir: &str) -> Result<()> {
        let started = Instant::now();
        // CPU-bound proof-of-work: keep it off the async worker threads
        let worker = self.clone();
        let solved = tokio::task::spawn_blocking(move || worker.solve())
            .await
            .map_err(|_| Error::blocked("anubis", BlockDetails::BotDetection))?;
        let Some((hash, nonce)) = solved else {
            return Err(Error::blocked("anubis", BlockDetails::BotDetection));
        };
        let url = parse::with_query(
            &format!("{origin}/.within.website/x/cmd/anubis/api/pass-challenge"),
            [
                ("id", self.id.as_str()),
                ("response", hash.as_str()),
                ("nonce", nonce.to_string().as_str()),
                ("redir", redir),
                (
                    "elapsedTime",
                    started.elapsed().as_millis().max(1).to_string().as_str(),
                ),
            ],
        );
        let resp = client.get(&url).await?;
        let status = resp.status();
        // Body is a tiny redirect page; read (and discard) it so the cookie
        // from the response headers is committed to the jar.
        drop(resp.bytes().await);
        if !status.is_success() {
            return Err(Error::blocked("anubis", BlockDetails::BotDetection));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERSTITIAL: &str = r#"<html><head>
<script id="anubis_version" type="application/json">"v1.25.0"</script>
<script id="anubis_challenge" type="application/json">{"rules":{"algorithm":"fast","difficulty":4},"challenge":{"issuedAt":"2026-08-22T20:04:49Z","metadata":{"User-Agent":"UA"},"id":"01a02b13-6f0b-717a-992e-2f334d9d0947","method":"fast","randomData":"e5c63e2a153f29181111f590e0930e985c8c1bab2c406385aa5bd8fb8f2ea505b1a7fbf5fe05fc0f66e512c12863e48df6627f82f78ed1fc7d37fcabd137d5f2","policyRuleHash":"ac980f49c4d35fab","difficulty":4,"spent":false}}</script>
</head><body>Making sure you are not a bot.</body></html>"#;

    #[test]
    fn extracts_challenge_from_interstitial() {
        assert!(Challenge::present_in(INTERSTITIAL));
        let c = Challenge::extract(INTERSTITIAL).expect("challenge parses");
        assert_eq!(c.id, "01a02b13-6f0b-717a-992e-2f334d9d0947");
        assert_eq!(c.difficulty, 4);
        assert!(c.random_data.starts_with("e5c63e2a"));
        assert!(!Challenge::present_in("<html>normal page</html>"));
        assert!(Challenge::extract("<html>normal page</html>").is_none());
    }

    #[test]
    fn extract_rejects_malformed_payloads() {
        for html in [
            r#"<script id="anubis_challenge" type="application/json">not json</script>"#,
            r#"<script id="anubis_challenge" type="application/json">{"rules":{"difficulty":4}}</script>"#,
            r#"<script id="anubis_challenge" type="application/json">{"rules":{"difficulty":4},"challenge":{"randomData":""}}</script>"#,
            r#"<script id="anubis_challenge" type="application/json">{"rules":{"difficulty":0},"challenge":{"randomData":"ab"}}</script>"#,
            r#"<script id="anubis_challenge" type="application/json">{"rules":{"difficulty":64},"challenge":{"randomData":"ab"}}</script>"#,
        ] {
            assert!(Challenge::extract(html).is_none(), "{html}");
        }
    }

    #[test]
    fn solve_finds_valid_proof_for_difficulty_4() {
        // difficulty 4 = two leading zero bytes (8 zero bits)
        let c = Challenge::extract(INTERSTITIAL).unwrap();
        let (hex, nonce) = c.solve().expect("solvable");
        assert_eq!(hex.len(), 64);
        let mut h = Sha256::new();
        h.update(c.random_data.as_bytes());
        h.update(nonce.to_string().as_bytes());
        let d = h.finalize();
        assert_eq!(
            &hex,
            &d.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        assert_eq!(&d[..2], &[0, 0], "digest {hex} lacks two zero bytes");
    }

    #[test]
    fn solve_is_minimal_nonce() {
        // the solver must return the smallest valid nonce
        let c = Challenge {
            id: String::new(),
            random_data: "deadbeef".into(),
            difficulty: 4,
        };
        let (_, nonce) = c.solve().expect("solvable");
        let mut h = Sha256::new();
        for smaller in 0..nonce {
            h.update(c.random_data.as_bytes());
            h.update(smaller.to_string().as_bytes());
            let d = h.finalize_reset();
            assert!(
                !(d[0] == 0 && d[1] == 0),
                "nonce {smaller} also satisfies; solver skipped it"
            );
        }
    }

    #[test]
    fn solve_handles_odd_difficulties() {
        // difficulty 3 = one zero byte + high nibble of the second byte
        // (higher odd difficulties need billions of hashes - not testable)
        for salt in ["a", "bb", "ccc", "dddd"] {
            let c = Challenge {
                id: String::new(),
                random_data: salt.into(),
                difficulty: 3,
            };
            let (hex, _) = c.solve().expect("solvable");
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                .collect();
            assert_eq!(bytes[0], 0);
            assert!(bytes[1] >> 4 == 0, "salt {salt}: {hex}");
        }
    }
}
