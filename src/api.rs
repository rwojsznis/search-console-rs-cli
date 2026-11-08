//! The Search Console endpoints the CLI calls, and nothing else (decision 1).
//!
//! `ApiRequest` has no public constructor: the only way to get one is a
//! method of `Api`, one per endpoint in `ENDPOINTS`. Every endpoint reads.
//! The two POSTs (Search Analytics and URL Inspection) are queries that
//! Google sends as POST because their parameters do not fit in a URL.

use crate::error::Error;
use crate::http::Method;
use crate::site::Site;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Map, Value, json};
use url::Url;

pub const DEFAULT_BASE_URL: &str = "https://searchconsole.googleapis.com/";

/// What `encodeURIComponent` leaves alone. Google's docs encode the site URL
/// this way: `https%3A%2F%2Fwww.example.com%2F`, `sc-domain%3Aexample.com`.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// One endpoint: its method and path template, with Google's placeholder
/// names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoint {
    pub method: Method,
    pub path: &'static str,
}

pub const SITES_LIST: Endpoint = Endpoint {
    method: Method::Get,
    path: "/webmasters/v3/sites",
};
pub const SITES_GET: Endpoint = Endpoint {
    method: Method::Get,
    path: "/webmasters/v3/sites/{siteUrl}",
};
pub const SITEMAPS_LIST: Endpoint = Endpoint {
    method: Method::Get,
    path: "/webmasters/v3/sites/{siteUrl}/sitemaps",
};
pub const SITEMAPS_GET: Endpoint = Endpoint {
    method: Method::Get,
    path: "/webmasters/v3/sites/{siteUrl}/sitemaps/{feedpath}",
};
pub const SEARCH_ANALYTICS_QUERY: Endpoint = Endpoint {
    method: Method::Post,
    path: "/webmasters/v3/sites/{siteUrl}/searchAnalytics/query",
};
pub const URL_INSPECTION: Endpoint = Endpoint {
    method: Method::Post,
    path: "/v1/urlInspection/index:inspect",
};

/// Every endpoint the CLI can call. Each one only reads.
pub const ENDPOINTS: [Endpoint; 6] = [
    SITES_LIST,
    SITES_GET,
    SITEMAPS_LIST,
    SITEMAPS_GET,
    SEARCH_ANALYTICS_QUERY,
    URL_INSPECTION,
];

/// A request to one of `ENDPOINTS`, ready to send.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiRequest {
    endpoint: Endpoint,
    url: Url,
    body: Option<Value>,
}

impl ApiRequest {
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    #[must_use]
    pub fn body(&self) -> Option<&Value> {
        self.body.as_ref()
    }

    /// `{"method", "url", "body"}`, for `--dry-run`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut value = Map::new();
        value.insert("method".into(), self.endpoint.method.as_str().into());
        value.insert("url".into(), self.url.as_str().into());
        if let Some(body) = &self.body {
            value.insert("body".into(), body.clone());
        }
        Value::Object(value)
    }
}

/// Checks `--base-url` once, before anything is sent, and gives it the
/// trailing slash the paths are joined to.
///
/// # Errors
///
/// A `config` error when the value does not parse, or its scheme is not http
/// or https.
pub fn parse_base_url(value: &str) -> Result<Url, Error> {
    let mut url = Url::parse(value)
        .map_err(|e| Error::config(format!("--base-url {value:?} is not a URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::config(format!(
            "--base-url {value:?} must be an http or https URL"
        )));
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

/// Builds the requests, against `--base-url`.
#[derive(Debug, Clone)]
pub struct Api {
    base: Url,
}

impl Api {
    #[must_use]
    pub fn new(base: Url) -> Self {
        Api { base }
    }

    /// # Errors
    ///
    /// Never in practice; see `build`.
    pub fn sites_list(&self) -> Result<ApiRequest, Error> {
        self.build(SITES_LIST, &[], &[], None)
    }

    /// # Errors
    ///
    /// Never in practice; see `build`.
    pub fn sites_get(&self, site: &Site) -> Result<ApiRequest, Error> {
        self.build(SITES_GET, &[site.as_str()], &[], None)
    }

    /// # Errors
    ///
    /// Never in practice; see `build`.
    pub fn sitemaps_list(
        &self,
        site: &Site,
        sitemap_index: Option<&str>,
    ) -> Result<ApiRequest, Error> {
        let query: Vec<(&str, &str)> = sitemap_index
            .map(|index| ("sitemapIndex", index))
            .into_iter()
            .collect();
        self.build(SITEMAPS_LIST, &[site.as_str()], &query, None)
    }

    /// # Errors
    ///
    /// Never in practice; see `build`.
    pub fn sitemaps_get(&self, site: &Site, feedpath: &str) -> Result<ApiRequest, Error> {
        self.build(SITEMAPS_GET, &[site.as_str(), feedpath], &[], None)
    }

    /// # Errors
    ///
    /// Never in practice; see `build`.
    pub fn search_analytics(&self, site: &Site, body: Value) -> Result<ApiRequest, Error> {
        self.build(SEARCH_ANALYTICS_QUERY, &[site.as_str()], &[], Some(body))
    }

    /// # Errors
    ///
    /// Never in practice; see `build`.
    pub fn inspect(
        &self,
        site: &Site,
        url: &str,
        language_code: Option<&str>,
    ) -> Result<ApiRequest, Error> {
        let mut body = json!({"inspectionUrl": url, "siteUrl": site.as_str()});
        if let Some(code) = language_code {
            body["languageCode"] = code.into();
        }
        self.build(URL_INSPECTION, &[], &[], Some(body))
    }

    /// Fills each `{...}` of the path template with the next value, encoded
    /// as one path segment.
    ///
    /// The values are a validated site name and an http(s) URL, so no value
    /// is empty, `.` or `..` (which URL parsing would resolve away, even
    /// encoded), and the result always parses. The error is there so that
    /// nothing here can panic.
    fn build(
        &self,
        endpoint: Endpoint,
        values: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<ApiRequest, Error> {
        let mut path = String::new();
        let mut values = values.iter();
        for (i, part) in endpoint.path.trim_start_matches('/').split('/').enumerate() {
            if i > 0 {
                path.push('/');
            }
            if part.starts_with('{') {
                let value = values.next().copied().unwrap_or_default();
                path.extend(utf8_percent_encode(value, COMPONENT));
            } else {
                path.push_str(part);
            }
        }
        let mut url = self
            .base
            .join(&path)
            .map_err(|e| Error::config(format!("cannot build the request URL: {e}")))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(ApiRequest {
            endpoint,
            url,
            body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api() -> Api {
        Api::new(parse_base_url(DEFAULT_BASE_URL).unwrap())
    }

    fn site(name: &str) -> Site {
        Site::parse(name).unwrap()
    }

    #[test]
    fn the_site_is_one_encoded_segment() {
        let url_prefix = api().sites_get(&site("https://www.example.com/")).unwrap();
        assert_eq!(
            url_prefix.url().as_str(),
            "https://searchconsole.googleapis.com/webmasters/v3/sites/https%3A%2F%2Fwww.example.com%2F"
        );
        let domain = api()
            .search_analytics(&site("sc-domain:example.com"), json!({}))
            .unwrap();
        assert_eq!(
            domain.url().as_str(),
            "https://searchconsole.googleapis.com/webmasters/v3/sites/sc-domain%3Aexample.com/searchAnalytics/query"
        );
        assert_eq!(domain.endpoint().method, Method::Post);
    }

    #[test]
    fn sitemaps_encode_the_feedpath_and_the_index() {
        let site = site("sc-domain:example.com");
        let get = api()
            .sitemaps_get(&site, "https://example.com/sitemap.xml")
            .unwrap();
        assert_eq!(
            get.url().path(),
            "/webmasters/v3/sites/sc-domain%3Aexample.com/sitemaps/https%3A%2F%2Fexample.com%2Fsitemap.xml"
        );
        let list = api()
            .sitemaps_list(&site, Some("https://example.com/index.xml"))
            .unwrap();
        assert_eq!(
            list.url().query(),
            Some("sitemapIndex=https%3A%2F%2Fexample.com%2Findex.xml")
        );
        assert_eq!(
            api().sitemaps_list(&site, None).unwrap().url().query(),
            None
        );
    }

    #[test]
    fn inspect_sends_the_site_and_the_url_in_the_body() {
        let request = api()
            .inspect(
                &site("sc-domain:example.com"),
                "https://example.com/a",
                Some("de"),
            )
            .unwrap();
        assert_eq!(
            request.to_json(),
            json!({
                "method": "POST",
                "url": "https://searchconsole.googleapis.com/v1/urlInspection/index:inspect",
                "body": {"inspectionUrl": "https://example.com/a", "siteUrl": "sc-domain:example.com", "languageCode": "de"}
            })
        );
        let bare = api()
            .inspect(
                &site("sc-domain:example.com"),
                "https://example.com/a",
                None,
            )
            .unwrap();
        assert!(bare.body().unwrap().get("languageCode").is_none());
    }

    #[test]
    fn a_get_has_no_body_in_its_dry_run() {
        assert_eq!(
            api().sites_list().unwrap().to_json(),
            json!({"method": "GET", "url": "https://searchconsole.googleapis.com/webmasters/v3/sites"})
        );
    }

    #[test]
    fn the_base_url_must_be_http_or_https_and_gets_a_slash() {
        assert_eq!(
            parse_base_url("http://127.0.0.1:1").unwrap().as_str(),
            "http://127.0.0.1:1/"
        );
        assert_eq!(
            parse_base_url("http://proxy/gsc").unwrap().as_str(),
            "http://proxy/gsc/"
        );
        let api = Api::new(parse_base_url("http://proxy/gsc").unwrap());
        assert_eq!(
            api.sites_list().unwrap().url().as_str(),
            "http://proxy/gsc/webmasters/v3/sites"
        );
        for bad in ["ftp://x", "not a url", "mailto:a@b"] {
            assert_eq!(
                parse_base_url(bad).unwrap_err().kind(),
                crate::error::Kind::Config,
                "{bad}"
            );
        }
    }

    #[test]
    fn only_two_endpoints_post_and_both_are_queries() {
        let posts: Vec<&str> = ENDPOINTS
            .iter()
            .filter(|e| e.method == Method::Post)
            .map(|e| e.path)
            .collect();
        assert_eq!(
            posts,
            [
                "/webmasters/v3/sites/{siteUrl}/searchAnalytics/query",
                "/v1/urlInspection/index:inspect"
            ]
        );
    }
}
