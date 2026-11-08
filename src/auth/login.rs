//! `auth login`: Google's OAuth flow for installed apps, with a loopback
//! redirect and PKCE (decision 16).
//!
//! The user brings the OAuth client (a "Desktop app" client secret from the
//! Google Cloud console); the CLI ships none. The result is an
//! `authorized_user` file in the same format gcloud writes, so either tool's
//! file works with the other's reader.

use crate::auth::{Env, SCOPE, TOKEN_URI};
use crate::errln;
use crate::error::Error;
use crate::http::{Body, Http, Method};
use crate::output::warn;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::digest::{SHA256, digest};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use url::Url;

const AUTH_URI: &str = "https://accounts.google.com/o/oauth2/auth";

/// What `auth login` was asked to do.
#[derive(Debug)]
pub struct Options {
    pub client_secret: PathBuf,
    /// Start a browser on the consent URL. The URL is printed either way.
    pub open_browser: bool,
    /// `$BROWSER`, when set: the program to start instead of the platform's.
    pub browser: Option<OsString>,
    /// How long to wait for the redirect.
    pub wait: Duration,
}

/// The OAuth client from a client secret file. Its secret is not
/// confidential for an installed app (Google says so), but it is still never
/// printed.
struct OAuthClient {
    client_id: String,
    client_secret: String,
    auth_uri: Url,
    token_uri: Url,
}

/// Runs the flow, writes the credentials file, and returns what `auth login`
/// prints.
///
/// # Errors
///
/// `config` for a bad client secret file or an unwritable config directory;
/// `auth` when the user or Google refuses; `timeout` when no redirect comes.
pub fn login(http: &Http, env: &Env, options: &Options) -> Result<Value, Error> {
    let target = env.login_file().ok_or_else(|| {
        Error::config("cannot find a config directory: set GSC_CONFIG_DIR, XDG_CONFIG_HOME or HOME")
    })?;
    let client = OAuthClient::read(&options.client_secret)?;
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr().map(|addr| (listener, addr.port())))
        .map_err(|e| Error::transport(format!("cannot listen on 127.0.0.1 for the redirect: {e}")));
    let (listener, port) = listener?;
    let redirect_uri = format!("http://127.0.0.1:{port}/");
    let verifier = random_text(32)?;
    let state = random_text(16)?;
    let challenge = URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes()));

    let mut url = client.auth_uri.clone();
    url.query_pairs_mut().extend_pairs([
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", state.as_str()),
        // A refresh token, every time: without `prompt=consent`, Google
        // sends one only on the first consent.
        ("access_type", "offline"),
        ("prompt", "consent"),
    ]);
    errln!(
        "{}",
        json!({"login": {
            "url": url.as_str(),
            "redirect_uri": redirect_uri,
            "message": format!(
                "Open the URL in a browser and allow access. Waiting up to {} s.",
                options.wait.as_secs()
            ),
        }})
    );
    let mut browser = options
        .open_browser
        .then(|| open_browser(url.as_str(), options.browser.as_ref()))
        .flatten();
    let code = wait_for_code(&listener, &state, options.wait);
    if let Some(child) = browser.as_mut() {
        // Reaps it when it has exited; a browser that is still running is
        // left alone.
        let _ = child.try_wait();
    }
    let code = code?;

    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("client_id", client.client_id.as_str()),
        ("client_secret", client.client_secret.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", verifier.as_str()),
    ];
    let answer = http.send(
        Method::Post,
        &client.token_uri,
        None,
        Body::Form(&form),
        Error::token,
    )?;
    let scope = answer.get("scope").and_then(Value::as_str);
    if let Some(scope) = scope
        && !scope.split(' ').any(|s| s == SCOPE)
    {
        return Err(
            Error::auth("Google did not grant access to Search Console data").with_hint(
                "Run `gsc-cli auth login` again, and tick the Search Console box on the \
                 consent screen.",
            ),
        );
    }
    let refresh_token = answer
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            Error::auth("Google answered without a refresh token").with_hint(
                "Remove the app's access at https://myaccount.google.com/permissions, then run \
                 `gsc-cli auth login` again.",
            )
        })?;
    let credentials = json!({
        "type": "authorized_user",
        "client_id": client.client_id,
        "client_secret": client.client_secret,
        "refresh_token": refresh_token,
        "token_uri": client.token_uri.as_str(),
    });
    write_private(&target, &credentials)?;
    Ok(json!({
        "credentials": target.display().to_string(),
        "type": "authorized_user",
        "client_id": client.client_id,
        "scope": scope,
    }))
}

impl OAuthClient {
    fn read(path: &Path) -> Result<OAuthClient, Error> {
        let bad =
            |why: &str| Error::config(format!("the client secret file {} {why}", path.display()));
        let text = fs::read_to_string(path).map_err(|e| bad(&format!("cannot be read: {e}")))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| bad(&format!("is not JSON: {e}")))?;
        let Some(installed) = value.get("installed").filter(|v| v.is_object()) else {
            let why = if value.get("web").is_some() {
                "is for a web application; create a client of type \"Desktop app\" instead"
            } else {
                "has no \"installed\" object: download the JSON of a \"Desktop app\" OAuth \
                 client from the Google Cloud console (APIs & Services, Credentials)"
            };
            return Err(bad(why));
        };
        let field = |name: &str| {
            installed
                .get(name)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| bad(&format!("has no installed.{name}")))
        };
        let url = |name: &str, default: &str| {
            let text = installed
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or(default);
            Url::parse(text)
                .ok()
                .filter(|url| matches!(url.scheme(), "http" | "https"))
                .ok_or_else(|| {
                    bad(&format!(
                        "has an installed.{name} that is not an http(s) URL"
                    ))
                })
        };
        Ok(OAuthClient {
            client_id: field("client_id")?,
            client_secret: field("client_secret")?,
            auth_uri: url("auth_uri", AUTH_URI)?,
            token_uri: url("token_uri", TOKEN_URI)?,
        })
    }
}

/// `bytes` random bytes, as URL-safe base64: the PKCE verifier (43
/// characters for 32 bytes, the RFC 7636 minimum) and the state.
fn random_text(bytes: usize) -> Result<String, Error> {
    let mut buffer = vec![0; bytes];
    SystemRandom::new()
        .fill(&mut buffer)
        .map_err(|_| Error::transport("the system random number generator failed"))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// Starts `$BROWSER`, or the platform's opener, on `url`. A failure is a
/// warning: the URL is already on stderr.
fn open_browser(url: &str, browser: Option<&OsString>) -> Option<Child> {
    let default = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let program = browser.cloned().unwrap_or_else(|| default.into());
    let started = Command::new(&program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match started {
        Ok(child) => Some(child),
        Err(e) => {
            warn(
                "browser",
                &format!(
                    "cannot start {}: {e}; open the URL by hand",
                    program.to_string_lossy()
                ),
            );
            None
        }
    }
}

/// Accepts connections on the redirect URI until one brings the code, an
/// error, or the wait runs out.
fn wait_for_code(listener: &TcpListener, state: &str, wait: Duration) -> Result<String, Error> {
    listener
        .set_nonblocking(true)
        .map_err(|e| Error::transport(format!("cannot wait for the redirect: {e}")))?;
    let deadline = Instant::now() + wait;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = answer_redirect(stream, state) {
                    return result;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(Error::timeout(wait.as_secs()).with_hint(
                        "Run `gsc-cli auth login` again and finish in the browser in time; \
                         --wait sets how long it waits.",
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                return Err(Error::transport(format!("cannot accept the redirect: {e}")));
            }
        }
    }
}

/// Answers one request to the redirect URI. `None` for a request that is not
/// the redirect (a browser also asks for `/favicon.ico`), so the wait goes
/// on.
fn answer_redirect(mut stream: TcpStream, state: &str) -> Option<Result<String, Error>> {
    // Accepted sockets inherit non-blocking mode on some platforms.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let params = request_params(&mut stream);
    let (status, page, result) = match params {
        Some(params) if params.contains_key("error") => (
            "400 Bad Request",
            "gsc-cli did not get access. You can close this tab.",
            Some(Err(Error::auth(format!(
                "the consent screen answered error={}",
                params["error"]
            )))),
        ),
        Some(params) if params.contains_key("code") => {
            if params.get("state").map(String::as_str) == Some(state) {
                (
                    "200 OK",
                    "gsc-cli is logged in. You can close this tab.",
                    Some(Ok(params["code"].clone())),
                )
            } else {
                (
                    "400 Bad Request",
                    "This answer is not for the running login. Run gsc-cli auth login again.",
                    Some(Err(Error::auth(
                        "the redirect's state does not match this login",
                    ))),
                )
            }
        }
        _ => ("404 Not Found", "Not found.", None),
    };
    let body = format!("<!doctype html><title>gsc-cli</title><p>{page}</p>\n");
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    result
}

/// The query parameters of a `GET` request line, or `None` for anything else.
fn request_params(stream: &mut TcpStream) -> Option<HashMap<String, String>> {
    let mut head = Vec::new();
    let mut chunk = [0; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 * 1024 {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&head);
    let mut line = head.lines().next()?.split(' ');
    if line.next()? != "GET" {
        return None;
    }
    let url = Url::parse("http://127.0.0.1/")
        .ok()?
        .join(line.next()?)
        .ok()?;
    Some(url.query_pairs().into_owned().collect())
}

/// Writes `value` readable by the user only: the file holds a refresh token.
/// Through a temporary file and a rename, so a crash never leaves half a file.
fn write_private(path: &Path, value: &Value) -> Result<(), Error> {
    let fail = |e: std::io::Error| Error::config(format!("cannot write {}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir).map_err(fail)?;
    }
    let temporary = path.with_extension("json.tmp");
    let _ = fs::remove_file(&temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temporary).map_err(fail)?;
    file.write_all(format!("{value:#}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(fail)?;
    fs::rename(&temporary, path).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Kind;

    fn client_file(dir: &Path, value: &Value) -> PathBuf {
        let path = dir.join("client.json");
        fs::write(&path, value.to_string()).unwrap();
        path
    }

    #[test]
    fn a_client_secret_needs_an_installed_client() {
        let dir = tempfile::tempdir().unwrap();
        let cases = [
            (json!({"web": {}}), "web application"),
            (json!({}), "Desktop app"),
            (
                json!({"installed": {"client_id": "a"}}),
                "installed.client_secret",
            ),
            (
                json!({"installed": {"client_id": "a", "client_secret": "b", "token_uri": "x"}}),
                "installed.token_uri",
            ),
        ];
        for (value, needle) in cases {
            let error = OAuthClient::read(&client_file(dir.path(), &value))
                .err()
                .unwrap();
            assert_eq!(error.kind(), Kind::Config);
            assert!(error.message().contains(needle), "{}", error.message());
        }
        let missing = OAuthClient::read(&dir.path().join("nope.json"))
            .err()
            .unwrap();
        assert!(missing.message().contains("cannot be read"));
        fs::write(dir.path().join("bad.json"), "{").unwrap();
        let bad = OAuthClient::read(&dir.path().join("bad.json"))
            .err()
            .unwrap();
        assert!(bad.message().contains("is not JSON"));
    }

    #[test]
    fn the_defaults_are_googles_endpoints() {
        let dir = tempfile::tempdir().unwrap();
        let path = client_file(
            dir.path(),
            &json!({"installed": {"client_id": "a", "client_secret": "b"}}),
        );
        let client = OAuthClient::read(&path).unwrap();
        assert_eq!(client.auth_uri.as_str(), AUTH_URI);
        assert_eq!(client.token_uri.as_str(), TOKEN_URI);
    }

    #[test]
    fn the_verifier_is_long_enough_for_pkce() {
        let verifier = random_text(32).unwrap();
        assert_eq!(verifier.len(), 43);
        assert_ne!(verifier, random_text(32).unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn the_credentials_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("credentials.json");
        write_private(&path, &json!({"x": 1})).unwrap();
        write_private(&path, &json!({"x": 2})).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let dir_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(dir_mode & 0o777, 0o700);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\n  \"x\": 2\n}\n");
    }

    #[test]
    fn an_unwritable_target_is_a_config_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "").unwrap();
        let error = write_private(&file.join("credentials.json"), &json!({})).unwrap_err();
        assert_eq!(error.kind(), Kind::Config);
    }

    #[test]
    fn a_browser_that_cannot_start_is_a_warning() {
        let missing = OsString::from("/nonexistent/browser");
        assert!(open_browser("https://example.com/", Some(&missing)).is_none());
    }
}
