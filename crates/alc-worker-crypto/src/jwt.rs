//! RS256 の JWT の署名 (汎用。claim は呼び手の型)。
//!
//! header は `{"alg":"RS256","typ":"JWT"}`、`kid` を渡せば `{"alg":"RS256","typ":"JWT","kid":"…"}`
//! (LINE の channel access token v2.1 の assertion が `kid` を要る。LINE WORKS の Service Account は要らない)。
//!
//! Private Key は PEM の文字列で受け、PKCS#1 (`BEGIN RSA PRIVATE KEY`) と PKCS#8 (`BEGIN PRIVATE KEY`) の両方を読む。
//! 前後の空白は落とし、実の改行が無く `\n` (2 文字) が在れば改行に直す ([`crate::secret::normalize_pem_newlines`])。
//! 写しの元は ippoan/alc-lineworks-worker の `crates/lineworks/src/jwt.rs` (9d76fd5) の署名の部分。
//!
//! エラーは段の名前だけ (鍵・claim を文に入れない)。

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::RsaPrivateKey;
use serde::Serialize;
use sha2::Sha256;

use crate::secret::normalize_pem_newlines;

/// 署名の失敗 (どの段で落ちたかだけ)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JwtError {
    /// Private Key の PEM が読めない
    Key,
    /// claim を JSON にできない
    Claims,
}

impl std::fmt::Display for JwtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Key => "key",
            Self::Claims => "claims",
        })
    }
}

/// header (`alg`・`typ`・任意の `kid` の順)
#[derive(Serialize)]
struct Header<'a> {
    alg: &'static str,
    typ: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kid: Option<&'a str>,
}

/// PEM (PKCS#1 か PKCS#8) を読む
fn parse_private_key(pem: &str) -> Result<RsaPrivateKey, JwtError> {
    let pem = normalize_pem_newlines(pem.trim());
    if pem.contains("BEGIN RSA PRIVATE KEY") {
        RsaPrivateKey::from_pkcs1_pem(&pem).map_err(|_| JwtError::Key)
    } else {
        RsaPrivateKey::from_pkcs8_pem(&pem).map_err(|_| JwtError::Key)
    }
}

/// `header.payload.signature` (どれも base64url・padding なし) を返す
pub fn sign_rs256<C: Serialize>(
    private_key_pem: &str,
    kid: Option<&str>,
    claims: &C,
) -> Result<String, JwtError> {
    let payload = serde_json::to_vec(claims).map_err(|_| JwtError::Claims)?;
    let key = parse_private_key(private_key_pem)?;
    let header = Header {
        alg: "RS256",
        typ: "JWT",
        kid,
    };
    // &str だけの struct なので JSON にするのは失敗しない
    let header = serde_json::to_vec(&header).unwrap_or_default();
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let signature = SigningKey::<Sha256>::new(key).sign(input.as_bytes());
    Ok(format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1::EncodeRsaPrivateKey;
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::signature::Verifier;
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    /// テスト用の鍵 (生成は重いので 1 回だけ。1024 bit = テスト専用の大きさ)
    fn test_key() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        KEY.get_or_init(|| RsaPrivateKey::new(&mut rand::thread_rng(), 1024).unwrap())
    }

    fn pkcs8_pem() -> String {
        test_key().to_pkcs8_pem(LineEnding::LF).unwrap().to_string()
    }

    fn pkcs1_pem() -> String {
        test_key().to_pkcs1_pem(LineEnding::LF).unwrap().to_string()
    }

    fn b64json(part: &str) -> serde_json::Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
    }

    /// 3 つに割り、署名を公開鍵で検証して (header, payload) を返す
    fn verify(jwt: &str) -> (serde_json::Value, serde_json::Value) {
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let sig =
            Signature::try_from(URL_SAFE_NO_PAD.decode(parts[2]).unwrap().as_slice()).unwrap();
        let verifier = VerifyingKey::<Sha256>::new(test_key().to_public_key());
        let input = format!("{}.{}", parts[0], parts[1]);
        verifier.verify(input.as_bytes(), &sig).unwrap();
        assert!(verifier.verify(b"other", &sig).is_err());
        (b64json(parts[0]), b64json(parts[1]))
    }

    /// LINE WORKS の Service Account の claim (alc-lineworks-worker の `jwt::Claims` と同じ形)
    #[derive(Serialize)]
    struct LineworksClaims {
        iss: String,
        sub: String,
        iat: u64,
        exp: u64,
    }

    /// LINE の channel access token v2.1 の claim (rust-alc-api の `alc_notify::clients::line::JwtClaims` と同じ形)
    #[derive(Serialize)]
    struct LineClaims {
        iss: String,
        sub: String,
        aud: String,
        exp: u64,
        token_exp: u64,
        token_type: String,
    }

    #[test]
    fn signs_verifiable_rs256_without_kid() {
        let claims = LineworksClaims {
            iss: "c".into(),
            sub: "s".into(),
            iat: 5,
            exp: 65,
        };
        let jwt = sign_rs256(&pkcs8_pem(), None, &claims).unwrap();
        let (header, payload) = verify(&jwt);
        // kid を渡さなければ header は alc-lineworks-worker と 1 byte も違わない
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(jwt.split('.').next().unwrap())
                .unwrap(),
            br#"{"alg":"RS256","typ":"JWT"}"#
        );
        assert_eq!(header, serde_json::json!({"alg": "RS256", "typ": "JWT"}));
        assert_eq!(
            payload,
            serde_json::json!({"iss": "c", "sub": "s", "iat": 5, "exp": 65})
        );
    }

    /// LINE の assertion (rust-alc-api の build_jwt_assertion と同じ header と claim) がこの関数と claim 型だけで組める
    #[test]
    fn signs_line_assertion_with_kid() {
        let now = 1_700_000_000;
        let claims = LineClaims {
            iss: "test-channel-id".into(),
            sub: "test-channel-id".into(),
            aud: "https://api.line.me/".into(),
            exp: now + 1800,
            token_exp: 60 * 60 * 24 * 30,
            token_type: "Bearer".into(),
        };
        let jwt = sign_rs256(&pkcs1_pem(), Some("test-key-id"), &claims).unwrap();
        let (header, payload) = verify(&jwt);
        assert_eq!(
            header,
            serde_json::json!({"alg": "RS256", "typ": "JWT", "kid": "test-key-id"})
        );
        assert_eq!(
            payload,
            serde_json::json!({
                "iss": "test-channel-id",
                "sub": "test-channel-id",
                "aud": "https://api.line.me/",
                "exp": now + 1800,
                "token_exp": 2_592_000,
                "token_type": "Bearer",
            })
        );
    }

    #[test]
    fn reads_pkcs1_and_pkcs8() {
        assert_eq!(&parse_private_key(&pkcs8_pem()).unwrap(), test_key());
        assert_eq!(&parse_private_key(&pkcs1_pem()).unwrap(), test_key());
    }

    #[test]
    fn reads_escaped_newlines_and_surrounding_space() {
        for pem in [pkcs8_pem(), pkcs1_pem()] {
            let escaped = format!("  {}  ", pem.trim().replace('\n', "\\n"));
            assert!(!escaped.contains('\n'));
            assert_eq!(&parse_private_key(&escaped).unwrap(), test_key());
            verify(&sign_rs256(&escaped, None, &serde_json::json!({"a": 1})).unwrap());
        }
    }

    /// 中身の無い偽の PEM (区切りの行は実行時に組み立てる — ソースに鍵の形の文字列を置かない)
    fn fake_pem(label: &str, body: &str, sep: &str) -> String {
        let d = "-".repeat(5);
        format!("{d}BEGIN {label}{d}{sep}{body}{sep}{d}END {label}{d}")
    }

    #[test]
    fn rejects_broken_keys() {
        assert_eq!(parse_private_key(""), Err(JwtError::Key));
        assert_eq!(
            parse_private_key(&fake_pem("RSA PRIVATE KEY", "AAAA", "\n")),
            Err(JwtError::Key)
        );
        assert_eq!(
            parse_private_key(&fake_pem("PRIVATE KEY", "AAAA", "\n")),
            Err(JwtError::Key)
        );
        assert_eq!(
            sign_rs256("not a pem", Some("k"), &serde_json::json!({})),
            Err(JwtError::Key)
        );
    }

    /// JSON にできない claim (文字列でない map の key) は Claims
    #[test]
    fn rejects_unserializable_claims() {
        let claims: BTreeMap<(u8, u8), u8> = [((1, 2), 3)].into();
        assert_eq!(
            sign_rs256(&pkcs8_pem(), None, &claims),
            Err(JwtError::Claims)
        );
        assert_eq!(JwtError::Key.to_string(), "key");
        assert_eq!(JwtError::Claims.to_string(), "claims");
    }
}
