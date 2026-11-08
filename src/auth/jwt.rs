//! The signed JWT a service account trades for an access token (RFC 7523;
//! Google's "Using OAuth 2.0 for Server to Server Applications").
//!
//! Hand-written on `ring`, which rustls already brings in: a JWT crate would
//! add a second RSA implementation, and the `rsa` crate has an advisory that
//! `cargo audit` fails on (decision 6).

use crate::error::Error;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::rand::SystemRandom;
use ring::signature::{RSA_PKCS1_SHA256, RsaKeyPair};
use serde_json::{Map, Value, json};

/// How long the assertion is valid. Google's maximum.
const LIFETIME: u64 = 3600;

/// The claims a service account asserts.
#[derive(Debug)]
pub(crate) struct Claims<'a> {
    pub(crate) client_email: &'a str,
    pub(crate) scope: &'a str,
    pub(crate) token_uri: &'a str,
    /// Seconds since the Unix epoch.
    pub(crate) now: u64,
}

/// `header.claims.signature`, signed RS256 with the service account key.
///
/// # Errors
///
/// An `auth` error when the key is not an RSA private key in PEM, or the
/// signing fails. The message never holds key material.
pub(crate) fn assertion(
    claims: &Claims,
    private_key_pem: &str,
    key_id: Option<&str>,
) -> Result<String, Error> {
    let key = key_pair(private_key_pem)?;
    let mut header = Map::new();
    header.insert("alg".into(), "RS256".into());
    header.insert("typ".into(), "JWT".into());
    if let Some(id) = key_id {
        header.insert("kid".into(), id.into());
    }
    let body = json!({
        "iss": claims.client_email,
        "scope": claims.scope,
        "aud": claims.token_uri,
        "iat": claims.now,
        "exp": claims.now + LIFETIME,
    });
    let mut message = encode(&Value::Object(header));
    message.push('.');
    message.push_str(&encode(&body));
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        message.as_bytes(),
        &mut signature,
    )
    .map_err(|_| Error::auth("cannot sign the service account JWT"))?;
    message.push('.');
    message.push_str(&URL_SAFE_NO_PAD.encode(signature));
    Ok(message)
}

fn encode(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(value.to_string())
}

/// Google's key files hold PKCS#8 (`BEGIN PRIVATE KEY`). PKCS#1 (`BEGIN RSA
/// PRIVATE KEY`) is accepted too, because some tools convert to it.
fn key_pair(pem: &str) -> Result<RsaKeyPair, Error> {
    let bad = |why: &str| Error::auth(format!("the service account private_key {why}"));
    let (label, der) = pem_body(pem).ok_or_else(|| bad("is not a PEM block"))?;
    let der = STANDARD
        .decode(der)
        .map_err(|_| bad("is not valid base64 inside its PEM block"))?;
    let parsed = match label.as_str() {
        "PRIVATE KEY" => RsaKeyPair::from_pkcs8(&der),
        "RSA PRIVATE KEY" => RsaKeyPair::from_der(&der),
        _ => return Err(bad("is not an RSA private key")),
    };
    parsed.map_err(|e| bad(&format!("is not a usable RSA key: {e}")))
}

/// The label and the base64 text of the first PEM block.
fn pem_body(pem: &str) -> Option<(String, String)> {
    let start = pem.find("-----BEGIN ")?;
    let rest = &pem[start + "-----BEGIN ".len()..];
    let (label, rest) = rest.split_once("-----")?;
    let end = rest.find(&format!("-----END {label}-----"))?;
    let body = rest[..end].split_whitespace().collect();
    Some((label.to_owned(), body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};

    const PKCS8: &str = include_str!("../../tests/fixtures/test-key-pkcs8.pem");
    const PKCS1: &str = include_str!("../../tests/fixtures/test-key-pkcs1.pem");

    fn claims() -> Claims<'static> {
        Claims {
            client_email: "bot@project.iam.gserviceaccount.com",
            scope: "https://www.googleapis.com/auth/webmasters.readonly",
            token_uri: "https://oauth2.googleapis.com/token",
            now: 1_700_000_000,
        }
    }

    fn part(jwt: &str, index: usize) -> Value {
        let text = jwt.split('.').nth(index).unwrap();
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(text).unwrap()).unwrap()
    }

    #[test]
    fn the_jwt_has_googles_claims_and_a_valid_signature() {
        let jwt = assertion(&claims(), PKCS8, Some("key-1")).unwrap();
        assert_eq!(
            part(&jwt, 0),
            json!({"alg": "RS256", "typ": "JWT", "kid": "key-1"})
        );
        assert_eq!(
            part(&jwt, 1),
            json!({
                "iss": "bot@project.iam.gserviceaccount.com",
                "scope": "https://www.googleapis.com/auth/webmasters.readonly",
                "aud": "https://oauth2.googleapis.com/token",
                "iat": 1_700_000_000_u64,
                "exp": 1_700_003_600_u64,
            })
        );
        let (message, signature) = jwt.rsplit_once('.').unwrap();
        let key = key_pair(PKCS8).unwrap();
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key.public().as_ref())
            .verify(
                message.as_bytes(),
                &URL_SAFE_NO_PAD.decode(signature).unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn a_pkcs1_key_signs_the_same_way_and_kid_is_optional() {
        let jwt = assertion(&claims(), PKCS1, None).unwrap();
        assert_eq!(part(&jwt, 0), json!({"alg": "RS256", "typ": "JWT"}));
        // PKCS#1 v1.5 signatures are deterministic: same key, same bytes.
        let other = assertion(&claims(), PKCS8, None).unwrap();
        assert_eq!(jwt, other);
    }

    #[test]
    fn a_bad_key_is_an_auth_error_without_the_key() {
        let cases = [
            ("not a key", "is not a PEM block"),
            (
                "-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----",
                "not valid base64",
            ),
            (
                "-----BEGIN EC PRIVATE KEY-----\nAAAA\n-----END EC PRIVATE KEY-----",
                "is not an RSA private key",
            ),
            (
                "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----",
                "is not a usable RSA key",
            ),
            (
                "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----",
                "is not a PEM block",
            ),
        ];
        for (pem, needle) in cases {
            let error = assertion(&claims(), pem, None).unwrap_err();
            assert_eq!(error.kind(), crate::error::Kind::Auth);
            assert!(
                error.message().contains(needle),
                "{pem}: {}",
                error.message()
            );
            assert!(!error.message().contains("AAAA"));
        }
    }
}
