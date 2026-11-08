//! Credentials: where they come from, and the access token they buy
//! (decision 6).
//!
//! Secrets (access tokens, refresh tokens, client secrets, private keys) never
//! appear in any output. The types that hold them have a hand-written `Debug`.

mod jwt;
pub mod login;

use crate::error::Error;
use crate::http::{Body, Http, Method};
use serde_json::{Map, Value, json};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

/// The one scope the CLI asks for. With it, Google refuses any write, even if
/// the CLI tried one (decision 1).
pub const SCOPE: &str = "https://www.googleapis.com/auth/webmasters.readonly";

/// Google's token endpoint, for credentials that do not name one.
pub const TOKEN_URI: &str = "https://oauth2.googleapis.com/token";

const JWT_GRANT: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

/// The file `auth login` writes, in the config directory.
const LOGIN_FILE: &str = "credentials.json";

/// Where the credentials came from, in the order they are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `--access-token` or `GSC_ACCESS_TOKEN`.
    AccessToken,
    /// `--credentials` or `GSC_CREDENTIALS`.
    Credentials,
    /// The file `gsc-cli auth login` wrote.
    Login,
    /// `GOOGLE_APPLICATION_CREDENTIALS`.
    GoogleApplicationCredentials,
    /// gcloud's application default credentials file.
    GcloudAdc,
}

impl Source {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Source::AccessToken => "access_token",
            Source::Credentials => "credentials",
            Source::Login => "login",
            Source::GoogleApplicationCredentials => "google_application_credentials",
            Source::GcloudAdc => "gcloud_adc",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Source::AccessToken => "--access-token or GSC_ACCESS_TOKEN",
            Source::Credentials => "--credentials or GSC_CREDENTIALS",
            Source::Login => "the file `gsc-cli auth login` wrote",
            Source::GoogleApplicationCredentials => "GOOGLE_APPLICATION_CREDENTIALS",
            Source::GcloudAdc => "gcloud's application default credentials",
        }
    }
}

/// The environment variables that decide where credentials are looked for,
/// read once. Tests build it by hand.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub gsc_config_dir: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub google_application_credentials: Option<PathBuf>,
    pub cloudsdk_config: Option<PathBuf>,
}

impl Env {
    #[must_use]
    pub fn from_process() -> Self {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|value: &OsString| !value.is_empty())
                .map(PathBuf::from)
        };
        Env {
            gsc_config_dir: var("GSC_CONFIG_DIR"),
            xdg_config_home: var("XDG_CONFIG_HOME"),
            home: var("HOME"),
            google_application_credentials: var("GOOGLE_APPLICATION_CREDENTIALS"),
            cloudsdk_config: var("CLOUDSDK_CONFIG"),
        }
    }

    /// `GSC_CONFIG_DIR`, else `$XDG_CONFIG_HOME/gsc-cli`, else
    /// `~/.config/gsc-cli` (on macOS too: a CLI's config is not an app's).
    #[must_use]
    pub fn config_dir(&self) -> Option<PathBuf> {
        self.gsc_config_dir.clone().or_else(|| {
            self.xdg_config_home
                .clone()
                .or_else(|| self.home.as_ref().map(|home| home.join(".config")))
                .map(|dir| dir.join("gsc-cli"))
        })
    }

    /// Where `auth login` writes, and where the credentials are looked for.
    #[must_use]
    pub fn login_file(&self) -> Option<PathBuf> {
        self.config_dir().map(|dir| dir.join(LOGIN_FILE))
    }

    fn gcloud_adc_file(&self) -> Option<PathBuf> {
        self.cloudsdk_config
            .clone()
            .or_else(|| {
                self.home
                    .as_ref()
                    .map(|home| home.join(".config").join("gcloud"))
            })
            .map(|dir| dir.join("application_default_credentials.json"))
    }
}

/// The two flags, and the environment.
#[derive(Clone, Default)]
pub struct AuthConfig {
    pub access_token: Option<String>,
    pub credentials: Option<PathBuf>,
    pub env: Env,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field("access_token", &self.access_token.as_ref().map(|_| "***"))
            .field("credentials", &self.credentials)
            .field("env", &self.env)
            .finish()
    }
}

impl AuthConfig {
    /// The first source that is set, in the order of decision 6. A set
    /// source that cannot be read is an error, not a reason to try the next
    /// one: falling through would use credentials the user did not choose.
    ///
    /// # Errors
    ///
    /// An `auth` error when no source is set, or the chosen one is not
    /// usable; a `config` error for an access token that is not a valid HTTP
    /// header value.
    pub fn resolve(&self) -> Result<Credentials, Error> {
        if let Some(token) = self.access_token.as_deref().filter(|t| !t.is_empty()) {
            if !token.bytes().all(|b| b.is_ascii_graphic()) {
                return Err(Error::config(
                    "the access token from --access-token or GSC_ACCESS_TOKEN is not a valid \
                     HTTP header value",
                ));
            }
            return Ok(Credentials {
                source: Source::AccessToken,
                path: None,
                kind: CredentialKind::AccessToken(token.to_owned()),
            });
        }
        if let Some(path) = &self.credentials {
            return Credentials::read(Source::Credentials, path);
        }
        if let Some(path) = self.env.login_file().filter(|p| p.exists()) {
            return Credentials::read(Source::Login, &path);
        }
        if let Some(path) = &self.env.google_application_credentials {
            return Credentials::read(Source::GoogleApplicationCredentials, path);
        }
        if let Some(path) = self.env.gcloud_adc_file().filter(|p| p.exists()) {
            return Credentials::read(Source::GcloudAdc, &path);
        }
        Err(Error::auth("no credentials found").with_hint(
            "Set GSC_CREDENTIALS to a service account key file, run `gsc-cli auth login`, or \
             set GSC_ACCESS_TOKEN to an access token with the webmasters.readonly scope. \
             `gsc-cli auth status` shows where gsc-cli looks.",
        ))
    }
}

/// Usable credentials, and where they came from.
pub struct Credentials {
    pub source: Source,
    pub path: Option<PathBuf>,
    kind: CredentialKind,
}

enum CredentialKind {
    AccessToken(String),
    ServiceAccount {
        client_email: String,
        private_key: String,
        private_key_id: Option<String>,
        token_uri: Url,
    },
    AuthorizedUser {
        client_id: String,
        client_secret: String,
        refresh_token: String,
        token_uri: Url,
    },
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("source", &self.source)
            .field("path", &self.path)
            .field("type", &self.type_name())
            .finish_non_exhaustive()
    }
}

/// An access token, and what the token endpoint said about it.
pub struct Token {
    pub access_token: String,
    pub scope: Option<String>,
    pub expires_in: Option<u64>,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("access_token", &"***")
            .field("scope", &self.scope)
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

impl Credentials {
    fn read(source: Source, path: &Path) -> Result<Credentials, Error> {
        let at = || format!("{} ({})", path.display(), source.describe());
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::auth(format!("cannot read the credentials file {}: {e}", at())))?;
        // serde_json's message names a line and column, never the content.
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| Error::auth(format!("the credentials file {} is not JSON: {e}", at())))?;
        let kind = parse(&value).map_err(|why| {
            let error = Error::auth(format!("the credentials file {} {why}", at()));
            if value.get("installed").is_some() || value.get("web").is_some() {
                error.with_hint(
                    "This is an OAuth client secret, not credentials: pass it to \
                     `gsc-cli auth login --client-secret`, which writes the credentials.",
                )
            } else {
                error
            }
        })?;
        Ok(Credentials {
            source,
            path: Some(path.to_owned()),
            kind,
        })
    }

    fn type_name(&self) -> &'static str {
        match self.kind {
            CredentialKind::AccessToken(_) => "access_token",
            CredentialKind::ServiceAccount { .. } => "service_account",
            CredentialKind::AuthorizedUser { .. } => "authorized_user",
        }
    }

    /// What `auth status` prints: no secret, only what identifies them.
    #[must_use]
    pub fn describe(&self) -> Value {
        let mut value = Map::new();
        value.insert("source".into(), self.source.as_str().into());
        if let Some(path) = &self.path {
            value.insert("path".into(), path.display().to_string().into());
        }
        value.insert("type".into(), self.type_name().into());
        match &self.kind {
            CredentialKind::AccessToken(_) => {}
            CredentialKind::ServiceAccount {
                client_email,
                token_uri,
                ..
            } => {
                value.insert("client_email".into(), client_email.clone().into());
                value.insert("token_uri".into(), token_uri.as_str().into());
            }
            CredentialKind::AuthorizedUser {
                client_id,
                token_uri,
                ..
            } => {
                value.insert("client_id".into(), client_id.clone().into());
                value.insert("token_uri".into(), token_uri.as_str().into());
            }
        }
        Value::Object(value)
    }

    /// The access token: as given, or from the token endpoint.
    ///
    /// # Errors
    ///
    /// An `auth` error when the token endpoint refuses, or the key cannot
    /// sign; `transport`, `timeout` or `decode` when no usable answer came.
    pub fn token(&self, http: &Http) -> Result<Token, Error> {
        match &self.kind {
            CredentialKind::AccessToken(token) => Ok(Token {
                access_token: token.clone(),
                scope: None,
                expires_in: None,
            }),
            CredentialKind::ServiceAccount {
                client_email,
                private_key,
                private_key_id,
                token_uri,
            } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let claims = jwt::Claims {
                    client_email,
                    scope: SCOPE,
                    token_uri: token_uri.as_str(),
                    now,
                };
                let assertion = jwt::assertion(&claims, private_key, private_key_id.as_deref())?;
                let form = [("grant_type", JWT_GRANT), ("assertion", assertion.as_str())];
                token_request(http, token_uri, &form)
            }
            CredentialKind::AuthorizedUser {
                client_id,
                client_secret,
                refresh_token,
                token_uri,
            } => {
                let form = [
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh_token.as_str()),
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                ];
                token_request(http, token_uri, &form)
            }
        }
    }
}

/// POSTs a form to the token endpoint and reads the token from the answer.
pub(crate) fn token_request(
    http: &Http,
    token_uri: &Url,
    form: &[(&str, &str)],
) -> Result<Token, Error> {
    let value = http.send(
        Method::Post,
        token_uri,
        None,
        Body::Form(form),
        Error::token,
    )?;
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty() && token.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or_else(|| {
            Error::decode("the token endpoint answered without a usable access_token")
                .at(Method::Post.as_str(), token_uri)
        })?;
    Ok(Token {
        access_token: access_token.to_owned(),
        scope: value
            .get("scope")
            .and_then(Value::as_str)
            .map(str::to_owned),
        expires_in: value.get("expires_in").and_then(Value::as_u64),
    })
}

impl Token {
    /// What `auth status` prints about the token.
    #[must_use]
    pub fn describe(&self) -> Value {
        json!({"scope": self.scope, "expires_in": self.expires_in})
    }

    /// Whether the token endpoint said the token covers `SCOPE` (or the
    /// read-write scope, which includes it). `None` when it did not say.
    #[must_use]
    pub fn has_scope(&self) -> Option<bool> {
        let full = SCOPE.trim_end_matches(".readonly");
        self.scope
            .as_deref()
            .map(|scope| scope.split(' ').any(|s| s == SCOPE || s == full))
    }
}

/// The credentials in a key file, or why there are none. Google's own JSON
/// field names.
fn parse(value: &Value) -> Result<CredentialKind, String> {
    let field = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("has no {name:?}"))
    };
    let token_uri = || {
        let text = value
            .get("token_uri")
            .and_then(Value::as_str)
            .unwrap_or(TOKEN_URI);
        Url::parse(text)
            .ok()
            .filter(|url| matches!(url.scheme(), "http" | "https"))
            .ok_or_else(|| format!("has a token_uri that is not an http or https URL: {text:?}"))
    };
    match value.get("type").and_then(Value::as_str) {
        Some("service_account") => Ok(CredentialKind::ServiceAccount {
            client_email: field("client_email")?,
            private_key: field("private_key")?,
            private_key_id: field("private_key_id").ok(),
            token_uri: token_uri()?,
        }),
        Some("authorized_user") => Ok(CredentialKind::AuthorizedUser {
            client_id: field("client_id")?,
            client_secret: field("client_secret")?,
            refresh_token: field("refresh_token")?,
            token_uri: token_uri()?,
        }),
        Some(other) => Err(format!(
            "has type {other:?}; gsc-cli reads service_account and authorized_user credentials"
        )),
        None => Err("has no \"type\"".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Kind;
    use std::fs;

    fn write(dir: &Path, name: &str, value: &Value) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, value.to_string()).unwrap();
        path
    }

    fn user() -> Value {
        json!({"type": "authorized_user", "client_id": "cid", "client_secret": "cs", "refresh_token": "rt"})
    }

    #[test]
    fn the_sources_are_tried_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let login_dir = home.join(".config").join("gsc-cli");
        let gcloud_dir = home.join(".config").join("gcloud");
        fs::create_dir_all(&login_dir).unwrap();
        fs::create_dir_all(&gcloud_dir).unwrap();
        let explicit = write(dir.path(), "explicit.json", &user());
        let gac = write(dir.path(), "gac.json", &user());
        let mut config = AuthConfig {
            access_token: Some("tok".into()),
            credentials: Some(explicit.clone()),
            env: Env {
                home: Some(home.clone()),
                google_application_credentials: Some(gac.clone()),
                ..Env::default()
            },
        };
        write(&login_dir, "credentials.json", &user());
        write(&gcloud_dir, "application_default_credentials.json", &user());

        let mut seen = Vec::new();
        for _ in 0..5 {
            let credentials = config.resolve().unwrap();
            seen.push(credentials.source);
            match credentials.source {
                Source::AccessToken => config.access_token = None,
                Source::Credentials => config.credentials = None,
                Source::Login => fs::remove_file(login_dir.join("credentials.json")).unwrap(),
                Source::GoogleApplicationCredentials => {
                    config.env.google_application_credentials = None;
                }
                Source::GcloudAdc => {
                    fs::remove_file(gcloud_dir.join("application_default_credentials.json"))
                        .unwrap();
                }
            }
        }
        assert_eq!(
            seen,
            [
                Source::AccessToken,
                Source::Credentials,
                Source::Login,
                Source::GoogleApplicationCredentials,
                Source::GcloudAdc
            ]
        );
        let error = config.resolve().unwrap_err();
        assert_eq!(error.kind(), Kind::Auth);
        assert!(error.hint().unwrap().contains("GSC_CREDENTIALS"));
    }

    #[test]
    fn every_source_names_itself_for_error_messages() {
        for (source, name, needle) in [
            (Source::AccessToken, "access_token", "GSC_ACCESS_TOKEN"),
            (Source::Credentials, "credentials", "GSC_CREDENTIALS"),
            (Source::Login, "login", "auth login"),
            (
                Source::GoogleApplicationCredentials,
                "google_application_credentials",
                "GOOGLE_APPLICATION_CREDENTIALS",
            ),
            (Source::GcloudAdc, "gcloud_adc", "gcloud"),
        ] {
            assert_eq!(source.as_str(), name);
            assert!(source.describe().contains(needle), "{name}");
        }
    }

    #[test]
    fn the_config_dir_follows_gsc_config_dir_then_xdg_then_home() {
        let mut env = Env {
            home: Some("/h".into()),
            ..Env::default()
        };
        assert_eq!(env.config_dir().unwrap(), Path::new("/h/.config/gsc-cli"));
        env.xdg_config_home = Some("/x".into());
        assert_eq!(env.config_dir().unwrap(), Path::new("/x/gsc-cli"));
        env.gsc_config_dir = Some("/g".into());
        assert_eq!(env.login_file().unwrap(), Path::new("/g/credentials.json"));
        env.cloudsdk_config = Some("/c".into());
        assert_eq!(
            env.gcloud_adc_file().unwrap(),
            Path::new("/c/application_default_credentials.json")
        );
        assert_eq!(Env::default().config_dir(), None);
    }

    #[test]
    fn a_chosen_source_that_cannot_be_read_is_an_error_not_a_fallthrough() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (None, "cannot read the credentials file"),
            (Some("{"), "is not JSON"),
            (
                Some(r#"{"type":"external_account"}"#),
                "has type \"external_account\"",
            ),
            (Some("{}"), "has no \"type\""),
            (
                Some(r#"{"type":"service_account","client_email":"a"}"#),
                "has no \"private_key\"",
            ),
            (
                Some(
                    r#"{"type":"authorized_user","client_id":"a","client_secret":"b","refresh_token":"c","token_uri":"ftp://x"}"#,
                ),
                "token_uri that is not an http or https URL",
            ),
        ];
        for (content, needle) in cases {
            let path = dir.path().join("creds.json");
            match content {
                Some(text) => fs::write(&path, text).unwrap(),
                None => drop(fs::remove_file(&path)),
            }
            let config = AuthConfig {
                credentials: Some(path),
                ..AuthConfig::default()
            };
            let error = config.resolve().unwrap_err();
            assert_eq!(error.kind(), Kind::Auth, "{needle}");
            assert!(error.message().contains(needle), "{}", error.message());
            assert!(
                error.message().contains("GSC_CREDENTIALS"),
                "{}",
                error.message()
            );
        }
    }

    #[test]
    fn a_client_secret_passed_as_credentials_gets_a_hint() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "client.json", &json!({"installed": {}}));
        let error = AuthConfig {
            credentials: Some(path),
            ..AuthConfig::default()
        }
        .resolve()
        .unwrap_err();
        assert!(error.hint().unwrap().contains("auth login --client-secret"));
    }

    #[test]
    fn an_access_token_must_be_a_header_value() {
        let config = AuthConfig {
            access_token: Some("bad token".into()),
            ..AuthConfig::default()
        };
        assert_eq!(config.resolve().unwrap_err().kind(), Kind::Config);
        // An empty one counts as not set.
        let empty = AuthConfig {
            access_token: Some(String::new()),
            ..AuthConfig::default()
        };
        assert_eq!(empty.resolve().unwrap_err().kind(), Kind::Auth);
    }

    #[test]
    fn describe_and_debug_never_show_a_secret() {
        let dir = tempfile::tempdir().unwrap();
        let account = json!({
            "type": "service_account",
            "client_email": "bot@p.iam.gserviceaccount.com",
            "private_key": "secret-key",
            "private_key_id": "kid",
        });
        for value in [account, user()] {
            let path = write(dir.path(), "c.json", &value);
            let credentials = AuthConfig {
                credentials: Some(path),
                ..AuthConfig::default()
            }
            .resolve()
            .unwrap();
            let shown = format!("{} {credentials:?}", credentials.describe());
            for secret in ["secret-key", "\"cs\"", "\"rt\""] {
                assert!(!shown.contains(secret), "{shown}");
            }
            assert!(shown.contains("token_uri"), "{shown}");
        }
        let config = AuthConfig {
            access_token: Some("tok-secret".into()),
            ..AuthConfig::default()
        };
        let credentials = config.resolve().unwrap();
        let shown = format!("{config:?} {credentials:?} {}", credentials.describe());
        assert!(!shown.contains("tok-secret"), "{shown}");
        let token = Token {
            access_token: "tok-secret".into(),
            scope: None,
            expires_in: None,
        };
        assert!(!format!("{token:?}").contains("tok-secret"));
    }

    #[test]
    fn the_scope_check_accepts_the_read_write_scope_too() {
        let token = |scope: Option<&str>| Token {
            access_token: String::new(),
            scope: scope.map(str::to_owned),
            expires_in: None,
        };
        assert_eq!(token(Some(SCOPE)).has_scope(), Some(true));
        assert_eq!(
            token(Some("openid https://www.googleapis.com/auth/webmasters")).has_scope(),
            Some(true)
        );
        assert_eq!(
            token(Some("https://www.googleapis.com/auth/cloud-platform")).has_scope(),
            Some(false)
        );
        assert_eq!(token(None).has_scope(), None);
    }
}
