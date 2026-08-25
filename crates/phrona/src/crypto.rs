//! Minimal HMAC-SHA-256 / PBKDF2-HMAC-SHA-256 (used by the ALTCHA
//! proof-of-work solver in `engines/altcha`). Standard definitions;
//! verified against RFC-distributed test vectors in the tests below.

#![allow(missing_docs)] // internal primitives; module docs suffice

use sha2::{Digest, Sha256};

const BLOCK: usize = 64;

/// A keyed HMAC-SHA-256 whose two padded key blocks are pre-compressed once.
/// Cloning the midstates makes each MAC cost just two finalizations - this
/// is what makes the ALTCHA proof-of-work scan fast.
#[derive(Clone)]
pub struct HmacSha256 {
    inner_mid: Sha256,
    outer_mid: Sha256,
}

impl HmacSha256 {
    pub fn new(key: &[u8]) -> Self {
        let mut k = [0u8; BLOCK];
        if key.len() > BLOCK {
            let mut d = Sha256::new();
            d.update(key);
            k[..32].copy_from_slice(&d.finalize());
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut inner_mid = Sha256::new();
        inner_mid.update(k.iter().map(|b| b ^ 0x36).collect::<Vec<u8>>());
        let mut outer_mid = Sha256::new();
        outer_mid.update(k.iter().map(|b| b ^ 0x5c).collect::<Vec<u8>>());
        Self {
            inner_mid,
            outer_mid,
        }
    }

    pub fn finish(&self, msg: &[u8]) -> [u8; 32] {
        let mut inner = self.inner_mid.clone();
        inner.update(msg);
        let ih = inner.finalize();
        let mut outer = self.outer_mid.clone();
        outer.update(ih);
        outer.finalize().into()
    }
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    HmacSha256::new(key).finish(msg)
}

/// PBKDF2-HMAC-SHA-256 writing `out.len()` derived bytes.
pub fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], rounds: u32, out: &mut [u8]) {
    let mac = HmacSha256::new(password);
    let mut block_index: u32 = 1;
    let mut offset = 0;
    while offset < out.len() {
        let mut msg = Vec::with_capacity(salt.len() + 4);
        msg.extend_from_slice(salt);
        msg.extend_from_slice(&block_index.to_be_bytes());

        let mut u = mac.finish(&msg);
        let mut t = u;
        for _ in 1..rounds {
            u = mac.finish(&u);
            for (ti, ui) in t.iter_mut().zip(u.iter()) {
                *ti ^= ui;
            }
        }
        let take = (out.len() - offset).min(32);
        out[offset..offset + take].copy_from_slice(&t[..take]);
        offset += take;
        block_index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    }

    #[test]
    fn rfc_vectors_pbkdf2_hmac_sha256() {
        // Widely-published PBKDF2-HMAC-SHA256 vectors (P="password",
        // S="salt").
        let cases: [(u32, &str); 3] = [
            (
                1,
                "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b",
            ),
            (
                2,
                "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43",
            ),
            (
                4096,
                "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a",
            ),
        ];
        for (rounds, want) in cases {
            let mut out = vec![0u8; 32];
            pbkdf2_hmac_sha256(b"password", b"salt", rounds, &mut out);
            assert_eq!(hex(&out), want, "rounds={rounds}");
        }
        // multi-block output: first block must equal the single-block result
        let mut out = vec![0u8; 33];
        pbkdf2_hmac_sha256(b"password", b"salt", 1, &mut out);
        assert_eq!(hex(&out[..32]), cases[0].1);
    }

    #[test]
    fn hmac_matches_known_vector() {
        // RFC 4231 test case 2: key="Jefe", data="what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
