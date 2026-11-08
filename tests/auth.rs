//! Credentials (decision 6) and `auth login` (decision 16), against a mock
//! token endpoint. No secret may appear in any output.

mod common;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{Run, SENTINEL_TOKEN, SITE_PATH, assert_no_secret, base_url, cli, expect_get, run};
use httptest::matchers::{ExecutionContext, KV, Matcher, all_of, contains, request, url_decoded};
use httptest::responders::status_code;
use httptest::{Expectation, Server};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Stdio};

const PRIVATE_KEY: &str = include_str!("fixtures/test-key-pkcs8.pem");
const REFRESH_TOKEN: &str = "refresh-sentinel-91b2";
const CLIENT_SECRET: &str = "client-secret-sentinel-4c1d";
const SECRETS: [&str; 3] = [REFRESH_TOKEN, CLIENT_SECRET, "BEGIN PRIVATE KEY"];

/// A form body that passes a test. httptest's closure matchers need a sized
/// input, and a decoded form is a slice.
type FormTest = dyn Fn(&[KV<str, str>]) -> bool + Send;

struct Form(Box<FormTest>);

impl Form {
    fn new(test: impl Fn(&[KV<str, str>]) -> bool + Send + 'static) -> Form {
        Form(Box::new(test))
    }
}

impl Matcher<[KV<str, str>]> for Form {
    fn matches(&mut self, input: &[KV<str, str>], _: &mut ExecutionContext) -> bool {
        (self.0)(input)
    }

    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("Form(..)")
    }
}

fn token_answer() -> String {
    json!({"access_token": SENTINEL_TOKEN, "expires_in": 3599, "token_type": "Bearer", "scope": "https://www.googleapis.com/auth/webmasters.readonly"})
        .to_string()
}

fn service_account(server: &Server) -> Value {
    json!({
        "type": "service_account",
        "project_id": "p",
        "private_key_id": "kid-1",
        "private_key": PRIVATE_KEY,
        "client_email": "bot@p.iam.gserviceaccount.com",
        "token_uri": server.url_str("/token"),
    })
}

fn authorized_user(server: &Server) -> Value {
    json!({
        "type": "authorized_user",
        "client_id": "cid.apps.googleusercontent.com",
        "client_secret": CLIENT_SECRET,
        "refresh_token": REFRESH_TOKEN,
        "token_uri": server.url_str("/token"),
    })
}

fn write(dir: &Path, name: &str, value: &Value) -> String {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, value.to_string()).unwrap();
    path.display().to_string()
}

/// The token endpoint, answering the refresh token grant.
fn expect_refresh(server: &Server) {
    server.expect(
        Expectation::matching(all_of![
            request::method_path("POST", "/token"),
            request::body(url_decoded(contains(("grant_type", "refresh_token")))),
            request::body(url_decoded(contains(("refresh_token", REFRESH_TOKEN)))),
            request::body(url_decoded(contains(("client_secret", CLIENT_SECRET)))),
        ])
        .respond_with(status_code(200).body(token_answer())),
    );
}

fn sites_list(server: &Server, env: &[(&str, &str)]) -> Run {
    run(&["sites", "list", "--base-url", &base_url(server)], env)
}

#[test]
fn a_service_account_signs_a_jwt_for_the_readonly_scope() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let key = write(dir.path(), "key.json", &service_account(&server));
    server.expect(
        Expectation::matching(all_of![
            request::method_path("POST", "/token"),
            request::body(url_decoded(contains((
                "grant_type",
                "urn:ietf:params:oauth:grant-type:jwt-bearer"
            )))),
            request::body(url_decoded(Form::new(|pairs: &[KV<str, str>]| {
                let Some(jwt) = pairs.iter().find(|kv| kv.k == "assertion").map(|kv| &kv.v) else {
                    return false;
                };
                let claims = jwt.split('.').nth(1).unwrap_or_default();
                let claims: Value =
                    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(claims).unwrap()).unwrap();
                claims["iss"] == "bot@p.iam.gserviceaccount.com"
                    && claims["scope"] == "https://www.googleapis.com/auth/webmasters.readonly"
                    && claims["aud"].as_str().unwrap().ends_with("/token")
            }))),
        ])
        .respond_with(status_code(200).body(token_answer())),
    );
    server.expect(
        expect_get("/webmasters/v3/sites", "")
            .respond_with(status_code(200).body(r#"{"siteEntry":[]}"#)),
    );

    let out = sites_list(&server, &[("GSC_CREDENTIALS", &key)]);

    assert_eq!(out.code, 0, "{out:?}");
    assert_no_secret(&out, &SECRETS);
}

#[test]
fn an_authorized_user_refreshes_its_token() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "user.json", &authorized_user(&server));
    expect_refresh(&server);
    server.expect(expect_get("/webmasters/v3/sites", "").respond_with(status_code(200).body("{}")));

    let out = run(
        &[
            "sites",
            "list",
            "--credentials",
            &file,
            "--base-url",
            &base_url(&server),
        ],
        &[],
    );

    assert_eq!(out.code, 0, "{out:?}");
    assert_no_secret(&out, &SECRETS);
}

#[test]
fn the_login_file_google_application_credentials_and_gcloud_are_found() {
    for source in ["login", "google_application_credentials", "gcloud_adc"] {
        let server = Server::run();
        let dir = tempfile::tempdir().unwrap();
        let user = authorized_user(&server);
        let config = dir.path().join("config").display().to_string();
        let gcloud = dir.path().join("gcloud").display().to_string();
        let env: Vec<(&str, String)> = match source {
            "login" => {
                write(dir.path(), "config/credentials.json", &user);
                vec![("GSC_CONFIG_DIR", config)]
            }
            "google_application_credentials" => vec![(
                "GOOGLE_APPLICATION_CREDENTIALS",
                write(dir.path(), "gac.json", &user),
            )],
            _ => {
                write(
                    dir.path(),
                    "gcloud/application_default_credentials.json",
                    &user,
                );
                vec![("CLOUDSDK_CONFIG", gcloud)]
            }
        };
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        expect_refresh(&server);
        server.expect(expect_get("/webmasters/v3/sites", "").respond_with(
            status_code(200).body(r#"{"siteEntry":[{"siteUrl":"sc-domain:example.com"}]}"#),
        ));

        let out = run(&["auth", "status", "--base-url", &base_url(&server)], &env);

        assert_eq!(out.code, 0, "{source}: {out:?}");
        let status = out.json();
        assert_eq!(status["credentials"]["source"], source);
        assert_eq!(status["credentials"]["type"], "authorized_user");
        assert_eq!(
            status["credentials"]["client_id"],
            "cid.apps.googleusercontent.com"
        );
        assert_eq!(status["token"]["expires_in"], 3599);
        assert_eq!(status["sites"], 1);
        assert_no_secret(&out, &SECRETS);
    }
}

#[test]
fn auth_status_with_an_access_token_and_dry_run() {
    let server = Server::run();
    server.expect(expect_get("/webmasters/v3/sites", "").respond_with(status_code(200).body("{}")));
    let out = common::run_api(&server, &["auth", "status"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.json(),
        json!({"credentials": {"source": "access_token", "type": "access_token"}, "sites": 0})
    );

    // --dry-run reads the credentials and sends nothing.
    let dir = tempfile::tempdir().unwrap();
    let key = write(dir.path(), "key.json", &service_account(&server));
    let dry = run(
        &["auth", "status", "--dry-run"],
        &[("GSC_CREDENTIALS", &key)],
    );
    assert_eq!(dry.code, 0, "{dry:?}");
    assert_eq!(
        dry.json()["credentials"]["client_email"],
        "bot@p.iam.gserviceaccount.com"
    );
    assert_no_secret(&dry, &SECRETS);
}

#[test]
fn auth_status_warns_about_a_token_without_the_scope() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "user.json", &authorized_user(&server));
    server.expect(
        Expectation::matching(request::method_path("POST", "/token")).respond_with(
            status_code(200).body(
                json!({"access_token": SENTINEL_TOKEN, "scope": "https://www.googleapis.com/auth/cloud-platform"})
                    .to_string(),
            ),
        ),
    );
    server.expect(
        expect_get("/webmasters/v3/sites", "").respond_with(status_code(403).body(
            json!({"error": {"code": 403, "message": "Request had insufficient authentication scopes.", "status": "PERMISSION_DENIED"}})
                .to_string(),
        )),
    );

    let out = run(
        &["auth", "status", "--base-url", &base_url(&server)],
        &[("GSC_CREDENTIALS", &file)],
    );

    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.warnings(), ["scope"]);
    assert!(
        out.error()["hint"]
            .as_str()
            .unwrap()
            .contains("webmasters.readonly")
    );
    assert_no_secret(&out, &SECRETS);
}

#[test]
fn a_refused_refresh_token_is_an_auth_error_at_the_token_endpoint() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "user.json", &authorized_user(&server));
    server.expect(
        Expectation::matching(request::method_path("POST", "/token"))
            .respond_with(status_code(400).body(
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
        )),
    );

    let out = sites_list(&server, &[("GSC_CREDENTIALS", &file)]);

    assert_eq!(out.code, 4, "{out:?}");
    assert_eq!(out.stdout, "");
    let error = out.error();
    assert_eq!(error["kind"], "auth");
    assert_eq!(error["reason"], "invalid_grant");
    assert_eq!(error["request"]["url"], server.url_str("/token"));
    assert_no_secret(&out, &SECRETS);
}

#[test]
fn a_token_answer_without_a_token_is_a_decode_error() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let file = write(dir.path(), "user.json", &authorized_user(&server));
    server.expect(
        Expectation::matching(request::method_path("POST", "/token"))
            .respond_with(status_code(200).body(r#"{"token_type":"Bearer"}"#)),
    );
    let out = sites_list(&server, &[("GSC_CREDENTIALS", &file)]);
    assert_eq!(out.code, 3, "{out:?}");
    assert_eq!(out.error()["kind"], "decode");
}

#[test]
fn a_bad_credentials_file_is_an_auth_error() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let mut account = service_account(&server);
    account["private_key"] = json!("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----");
    let file = write(dir.path(), "bad.json", &account);
    let out = sites_list(&server, &[("GSC_CREDENTIALS", &file)]);
    assert_eq!(out.code, 4, "{out:?}");
    assert!(
        out.error()["message"]
            .as_str()
            .unwrap()
            .contains("private_key")
    );
    // `auth status` stops at the same place.
    let status = run(
        &["auth", "status"],
        &[("GSC_CREDENTIALS", "/nonexistent.json")],
    );
    assert_eq!(status.code, 4, "{status:?}");
}

// ---- auth login --------------------------------------------------------

fn client_secret(dir: &Path, server: &Server) -> String {
    write(
        dir,
        "client_secret.json",
        &json!({"installed": {
            "client_id": "cid.apps.googleusercontent.com",
            "client_secret": CLIENT_SECRET,
            "auth_uri": "https://accounts.google.com/o/oauth2/auth",
            "token_uri": server.url_str("/token"),
            "redirect_uris": ["http://localhost"],
        }}),
    )
}

/// A running `auth login`, and the URL it printed.
struct Login {
    child: Child,
    stderr: BufReader<std::process::ChildStderr>,
    url: url::Url,
}

impl Login {
    fn start(config: &Path, client: &str, extra: &[&str], env: &[(&str, &str)]) -> Login {
        let mut cmd = cli();
        cmd.args(["auth", "login", "--client-secret", client])
            .args(extra)
            .env("GSC_CONFIG_DIR", config)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        let login: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
        let url = url::Url::parse(login["login"]["url"].as_str().unwrap()).unwrap();
        Login { child, stderr, url }
    }

    fn param(&self, name: &str) -> String {
        self.url.query_pairs().find(|(k, _)| k == name).map_or_else(
            || panic!("no {name} in {}", self.url),
            |(_, v)| v.into_owned(),
        )
    }

    /// Sends what the browser would after the consent screen, and returns
    /// the page the CLI answered with.
    fn redirect(&self, query: &str) -> String {
        let redirect = url::Url::parse(&self.param("redirect_uri")).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", redirect.port().unwrap())).unwrap();
        write!(stream, "GET /?{query} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
        let mut page = String::new();
        stream.read_to_string(&mut page).unwrap();
        page
    }

    /// Connects to the redirect URI and sends `text` as it is.
    fn raw(&self, text: &str) {
        let redirect = url::Url::parse(&self.param("redirect_uri")).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", redirect.port().unwrap())).unwrap();
        stream.write_all(text.as_bytes()).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut page = String::new();
        stream.read_to_string(&mut page).unwrap();
        assert!(page.starts_with("HTTP/1.1 404"), "{page}");
    }

    fn finish(mut self) -> Run {
        let mut rest = String::new();
        self.stderr.read_to_string(&mut rest).unwrap();
        let out = self.child.wait_with_output().unwrap();
        Run {
            code: out.status.code().unwrap(),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: rest,
        }
    }
}

#[test]
fn login_runs_the_pkce_flow_and_writes_a_private_authorized_user_file() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let client = client_secret(dir.path(), &server);
    let login = Login::start(&config, &client, &["--no-browser"], &[]);

    assert_eq!(login.url.host_str(), Some("accounts.google.com"));
    assert_eq!(
        login.param("scope"),
        "https://www.googleapis.com/auth/webmasters.readonly"
    );
    assert_eq!(login.param("code_challenge_method"), "S256");
    assert_eq!(login.param("access_type"), "offline");
    let challenge = login.param("code_challenge");
    let redirect_uri = login.param("redirect_uri");
    server.expect(
        Expectation::matching(all_of![
            request::method_path("POST", "/token"),
            request::body(url_decoded(contains(("grant_type", "authorization_code")))),
            request::body(url_decoded(contains(("code", "the-code")))),
            request::body(url_decoded(contains(("redirect_uri", redirect_uri)))),
            request::body(url_decoded(Form::new(move |pairs: &[KV<str, str>]| {
                pairs.iter().any(|kv| {
                    let digest = ring::digest::digest(&ring::digest::SHA256, kv.v.as_bytes());
                    kv.k == "code_verifier" && URL_SAFE_NO_PAD.encode(digest) == challenge
                })
            }))),
        ])
        .respond_with(status_code(200).body(
            json!({"access_token": "a", "refresh_token": REFRESH_TOKEN, "scope": "https://www.googleapis.com/auth/webmasters.readonly"})
                .to_string(),
        )),
    );

    // A browser asks for the favicon too, and anything else is not the
    // redirect either; the wait goes on.
    assert!(login.redirect("").starts_with("HTTP/1.1 404"));
    login.raw("POST /?code=x HTTP/1.1\r\n\r\n");
    login.raw("");
    let state = login.param("state");
    let page = login.redirect(&format!("state={state}&code=the-code&scope=x"));
    assert!(page.contains("close this tab"), "{page}");
    let out = login.finish();

    assert_eq!(out.code, 0, "{out:?}");
    let file = config.join("credentials.json");
    assert_eq!(out.json()["credentials"], file.display().to_string());
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(saved["type"], "authorized_user");
    assert_eq!(saved["refresh_token"], REFRESH_TOKEN);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert_no_secret(&out, &SECRETS);

    // The file is found by the next command.
    expect_refresh(&server);
    server.expect(expect_get(SITE_PATH, "").respond_with(status_code(200).body("{}")));
    let next = run(
        &[
            "sites",
            "get",
            "-s",
            common::SITE,
            "--base-url",
            &base_url(&server),
        ],
        &[("GSC_CONFIG_DIR", &config.display().to_string())],
    );
    assert_eq!(next.code, 0, "{next:?}");
}

#[test]
fn login_fails_cleanly_when_access_is_refused_or_the_state_is_wrong() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let client = client_secret(dir.path(), &server);
    for (query, needle) in [
        ("error=access_denied", "error=access_denied"),
        ("state=forged&code=x", "state does not match"),
    ] {
        let login = Login::start(dir.path(), &client, &["--no-browser"], &[]);
        let page = login.redirect(query);
        assert!(page.starts_with("HTTP/1.1 400"), "{page}");
        let out = login.finish();
        assert_eq!(out.code, 4, "{out:?}");
        assert!(
            out.error()["message"].as_str().unwrap().contains(needle),
            "{out:?}"
        );
        assert!(!dir.path().join("credentials.json").exists());
    }
}

#[test]
fn login_checks_what_google_granted() {
    let mut server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let client = client_secret(dir.path(), &server);
    let answers = [
        (
            json!({"access_token": "a", "scope": "openid"}),
            "did not grant",
        ),
        (json!({"access_token": "a"}), "without a refresh token"),
    ];
    for (answer, needle) in answers {
        server.expect(
            Expectation::matching(request::method_path("POST", "/token"))
                .respond_with(status_code(200).body(answer.to_string())),
        );
        let login = Login::start(dir.path(), &client, &["--no-browser"], &[]);
        let state = login.param("state");
        login.redirect(&format!("state={state}&code=c"));
        let out = login.finish();
        assert_eq!(out.code, 4, "{out:?}");
        assert!(
            out.error()["message"].as_str().unwrap().contains(needle),
            "{out:?}"
        );
        server.verify_and_clear();
    }
}

#[test]
fn login_starts_browser_and_gives_up_after_wait() {
    let server = Server::run();
    let dir = tempfile::tempdir().unwrap();
    let client = client_secret(dir.path(), &server);
    // `true` stands in for a browser: it starts, and does nothing.
    let login = Login::start(
        dir.path(),
        &client,
        &["--wait", "1"],
        &[("BROWSER", "true")],
    );
    let out = login.finish();
    assert_eq!(out.code, 3, "{out:?}");
    let error = out.error();
    assert_eq!(error["kind"], "timeout");
    assert!(error["hint"].as_str().unwrap().contains("--wait"));
}

#[test]
fn login_needs_a_desktop_client_a_config_dir_and_no_dry_run() {
    let dir = tempfile::tempdir().unwrap();
    let web = write(dir.path(), "web.json", &json!({"web": {}}));
    let out = run(
        &["auth", "login", "--client-secret", &web],
        &[("GSC_CONFIG_DIR", &dir.path().display().to_string())],
    );
    assert_eq!(out.code, 2, "{out:?}");
    assert!(
        out.error()["message"]
            .as_str()
            .unwrap()
            .contains("Desktop app")
    );

    let dry = run(
        &["auth", "login", "--client-secret", &web, "--dry-run"],
        &[],
    );
    assert_eq!(dry.code, 2, "{dry:?}");

    let mut cmd = cli();
    cmd.env_remove("HOME")
        .args(["auth", "login", "--client-secret", &web]);
    let homeless = cmd.output().unwrap();
    assert_eq!(homeless.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&homeless.stderr).contains("config directory"));
}
