//! [ALTCHA](https://altcha.org) proof-of-work solver (as used by Mojeek's
//! search challenge).
//!
//! Flow observed on the wire (2026-08 capture):
//!
//! 1. `GET /search` answers 200 with a challenge page (`.captcha-wrap`)
//!    when the client is unverified.
//! 2. The widget fetches `/captcha/challenge`, which returns
//!    `{"parameters":{algorithm,cost,keyLength,keyPrefix,nonce,salt,
//!    keySignature,expiresAt},"signature":<hex>}`.
//! 3. The client searches for the counter the server used:
//!    `PBKDF2-HMAC-SHA-256(password = nonce_bytes || be_u32(counter),
//!    salt = salt_bytes, iterations = cost, len = keyLength)` must start
//!    with `key_prefix_bytes`. Servers draw a small counter, so the scan
//!    from 0 terminates quickly.
//! 4. The widget submits base64(JSON) via multipart form field `altcha`
//!    to `/captcha/verify`; success returns
//!    `{"ok":true,"verified":true}` and sets the `chllg` clearance
//!    cookie, after which normal searches pass.

use crate::crypto::pbkdf2_hmac_sha256;
use crate::error::{BlockDetails, Error};
use base64::Engine as _;

/// Hard cap on scanned counters: servers draw small counters; anything past
/// this is either a policy change or abuse.
pub const MAX_COUNTER: u32 = 5_000_000;

/// A parsed challenge document.
#[derive(Debug, Clone)]
pub struct Challenge {
    /// Full `parameters` object (resubmitted verbatim in the solution).
    pub parameters: serde_json::Value,
    /// Server signature over the parameters.
    pub signature: String,
    // unpacked fields
    nonce: Vec<u8>,
    salt: Vec<u8>,
    key_prefix: Vec<u8>,
    cost: u32,
    key_length: usize,
}

impl Challenge {
    /// Parse a challenge descriptor from the engine's JSON payload.
    pub fn parse(json: &serde_json::Value) -> Option<Self> {
        let p = json.get("parameters")?;
        let hexdec = |s: &str| -> Option<Vec<u8>> {
            if s.len() % 2 != 0 {
                return None;
            }
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
                .collect()
        };
        let nonce = hexdec(p.get("nonce")?.as_str()?)?;
        let salt = hexdec(p.get("salt")?.as_str()?)?;
        let key_prefix = hexdec(p.get("keyPrefix")?.as_str()?)?;
        if key_prefix.is_empty() {
            return None;
        }
        let cost = p.get("cost")?.as_u64()? as u32;
        if cost == 0 || cost > 100_000 {
            return None;
        }
        let key_length = p.get("keyLength").and_then(|v| v.as_u64()).unwrap_or(32) as usize;
        if key_length == 0 || key_length > 128 {
            return None;
        }
        Some(Self {
            parameters: p.clone(),
            signature: json.get("signature")?.as_str()?.to_string(),
            nonce,
            salt,
            key_prefix,
            cost,
            key_length,
        })
    }

    fn derive(&self, password: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; self.key_length];
        pbkdf2_hmac_sha256(password, &self.salt, self.cost, &mut out);
        out
    }

    /// Find the server's counter: smallest `n` with
    /// `PBKDF2(nonce || be_u32(n))` starting with `keyPrefix`. Returns
    /// `(counter, hex(derived_key))`.
    pub fn solve(&self) -> Option<(u32, String)> {
        let mut pw = Vec::with_capacity(self.nonce.len() + 4);
        pw.extend_from_slice(&self.nonce);
        let fixed = pw.len();
        for counter in 0..MAX_COUNTER {
            pw.truncate(fixed);
            pw.extend_from_slice(&counter.to_be_bytes());
            let key = self.derive(&pw);
            if key.starts_with(&self.key_prefix) {
                let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
                return Some((counter, hex));
            }
        }
        None
    }

    /// Build the base64 payload submitted as the `altcha` form field.
    /// Mojeek's widget submits the *nested* format
    /// (`{"challenge":{parameters,signature},"solution":{...}}`), captured
    /// on the wire 2026-08.
    pub fn solution_payload(&self, number: u32, derived_key_hex: &str, took_ms: f64) -> String {
        let payload = serde_json::json!({
            "challenge": {
                "parameters": self.parameters,
                "signature": self.signature,
            },
            "solution": {
                "counter": number,
                "derivedKey": derived_key_hex,
                "time": took_ms,
            },
        });
        base64::engine::general_purpose::STANDARD.encode(payload.to_string())
    }
}

/// Multipart body carrying only the `altcha` field, mirroring the widget's
/// `FormData(form)` submission.
pub fn verify_body(payload_b64: &str) -> (String, String) {
    let boundary = format!(
        "----phrona-altcha{}",
        crate::engines::util::random_token(16)
    );
    let body = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"altcha\"\r\n\r\n{p}\r\n--{b}--\r\n",
        b = boundary,
        p = payload_b64
    );
    (boundary, body)
}

/// Convenience error for an unsolvable/invalid challenge.
pub fn blocked_error(engine: &'static str) -> Error {
    Error::blocked(engine, BlockDetails::Captcha)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a challenge exactly like Mojeek's server does: pick the
    /// counter first, derive the key, publish its prefix.
    fn make_challenge(counter: u32) -> serde_json::Value {
        // low cost keeps the scan fast; the algorithm path is identical
        make_challenge_with_cost(counter, 100)
    }

    fn make_challenge_with_cost(counter: u32, cost: u32) -> serde_json::Value {
        let nonce_hex = "1215d5e21e64fbaf5ed229d3270884ed";
        let salt_hex = "4a43a505d27582474de64f5d66f00f69";
        let nonce = hexdec(nonce_hex);
        let salt = hexdec(salt_hex);
        let mut pw = nonce.clone();
        pw.extend_from_slice(&counter.to_be_bytes());
        let mut dk = vec![0u8; 32];
        pbkdf2_hmac_sha256(&pw, &salt, cost, &mut dk);
        serde_json::json!({
            "parameters": {
                "algorithm": "PBKDF2/SHA-256",
                "cost": cost,
                "keyLength": 32,
                "keyPrefix": hexs(&dk[..16]),
                "nonce": nonce_hex,
                "salt": salt_hex,
                "keySignature": hexs(&dk),
                "expiresAt": 1787485351u64
            },
            "signature": "ed23e96b"
        })
    }

    fn hexdec(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn hexs(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn parses_and_solves_real_shape() {
        let want_counter = 1337;
        let json = make_challenge(want_counter);
        let c = Challenge::parse(&json).expect("parses");
        let (n, derived) = c.solve().expect("solvable");
        assert_eq!(n, want_counter);
        // the submitted derivedKey must equal the full derivation
        assert!(derived.starts_with(c.parameters["keyPrefix"].as_str().unwrap()));
        let payload = c.solution_payload(n, &derived, 392.6);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&payload)
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        // nested format as captured from Mojeek's widget
        assert_eq!(
            v["challenge"]["parameters"]["nonce"],
            json["parameters"]["nonce"]
        );
        assert_eq!(v["challenge"]["signature"], json["signature"]);
        assert_eq!(v["solution"]["counter"], want_counter);
        assert_eq!(v["solution"]["derivedKey"], derived);
    }

    #[test]
    fn solves_real_world_cost_quickly() {
        // server drew counter=283 in the captured session; reproduce that
        let json = make_challenge_with_cost(283, 8000);
        let c = Challenge::parse(&json).expect("parses");
        assert_eq!(c.solve().map(|(n, _)| n), Some(283));
    }

    #[test]
    fn parse_rejects_malformed() {
        for bad in [
            serde_json::json!({}),
            serde_json::json!({"parameters": {}, "signature": "ab"}),
            serde_json::json!({"parameters": {"algorithm":"PBKDF2/SHA-256","cost":0,
                "keyPrefix":"aa","nonce":"aabb","salt":"ccdd"}, "signature": "ab"}),
            serde_json::json!({"parameters": {"algorithm":"PBKDF2/SHA-256","cost":900000,
                "keyPrefix":"aa","nonce":"aabb","salt":"ccdd"}, "signature": "ab"}),
        ] {
            assert!(Challenge::parse(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn verify_body_is_multipart() {
        let (boundary, body) = verify_body("QUJD");
        assert!(body.starts_with(&format!("--{boundary}\r\n")));
        assert!(body.contains("Content-Disposition: form-data; name=\"altcha\"\r\n\r\nQUJD\r\n"));
        assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    }
}
