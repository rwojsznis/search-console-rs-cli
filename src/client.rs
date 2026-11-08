//! Sends `ApiRequest`s with an access token. The credentials are read and
//! the token fetched on the first request, once per run, so `--dry-run` and
//! a command that fails validation never touch them (decision 6).

use crate::api::ApiRequest;
use crate::auth::{AuthConfig, Credentials, Token};
use crate::error::Error;
use crate::http::{Body, Http};
use serde_json::Value;

#[derive(Debug)]
pub struct Client {
    http: Http,
    auth: AuthConfig,
    credentials: Option<Credentials>,
    token: Option<Token>,
}

impl Client {
    #[must_use]
    pub fn new(http: Http, auth: AuthConfig) -> Self {
        Client {
            http,
            auth,
            credentials: None,
            token: None,
        }
    }

    #[must_use]
    pub fn http(&self) -> &Http {
        &self.http
    }

    #[must_use]
    pub fn auth(&self) -> &AuthConfig {
        &self.auth
    }

    /// The credentials, read on first use.
    ///
    /// # Errors
    ///
    /// See `AuthConfig::resolve`.
    pub fn credentials(&mut self) -> Result<&Credentials, Error> {
        let credentials = match self.credentials.take() {
            Some(credentials) => credentials,
            None => self.auth.resolve()?,
        };
        Ok(self.credentials.insert(credentials))
    }

    /// The access token, fetched on first use.
    ///
    /// # Errors
    ///
    /// See `AuthConfig::resolve` and `Credentials::token`.
    pub fn token(&mut self) -> Result<&Token, Error> {
        let token = if let Some(token) = self.token.take() {
            token
        } else {
            let credentials = match self.credentials.take() {
                Some(credentials) => credentials,
                None => self.auth.resolve()?,
            };
            let token = credentials.token(&self.http);
            self.credentials = Some(credentials);
            token?
        };
        Ok(self.token.insert(token))
    }

    /// Sends one request and returns its JSON.
    ///
    /// # Errors
    ///
    /// Any error of decision 5. A credentials error carries this request,
    /// unless it already names the token endpoint.
    pub fn send(&mut self, request: &ApiRequest) -> Result<Value, Error> {
        let method = request.endpoint().method;
        let at = |error: Error| error.at(method.as_str(), request.url());
        let bearer = self.token().map_err(at)?.access_token.clone();
        let body = request.body().map_or(Body::None, Body::Json);
        self.http
            .send(method, request.url(), Some(&bearer), body, Error::api)
    }
}
