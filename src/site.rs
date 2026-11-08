//! A Search Console property, as the API names it (decision 8).

use url::Url;

/// `sc-domain:example.com` (a domain property) or `https://example.com/` (a
/// URL-prefix property). Agents mix the two up, and a bare `example.com`
/// could be either, so it is rejected with both spellings rather than
/// guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site(String);

impl Site {
    /// Checks and normalizes a property name: the domain in lowercase, and a
    /// URL as `Url` serializes it (lowercase host, `/` for an empty path).
    /// The API matches the name exactly, so `https://example.com` must
    /// become `https://example.com/`.
    ///
    /// # Errors
    ///
    /// The text for clap, when `value` is neither form.
    pub fn parse(value: &str) -> Result<Site, String> {
        let value = value.trim();
        if let Some(domain) = value.strip_prefix("sc-domain:") {
            if domain.is_empty() || domain.contains(['/', ':', ' ']) {
                return Err(format!(
                    "{value:?} is not a domain property: write sc-domain:example.com"
                ));
            }
            return Ok(Site(format!("sc-domain:{}", domain.to_ascii_lowercase())));
        }
        if let Ok(url) = Url::parse(value)
            && matches!(url.scheme(), "http" | "https")
            && url.host().is_some()
        {
            return Ok(Site(url.to_string()));
        }
        Err(format!(
            "{value:?} is not a Search Console property: write sc-domain:example.com for a \
             domain property, or https://example.com/ for a URL-prefix property \
             (`gsc-cli sites list` prints yours)"
        ))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An http or https URL, for `inspect` and `sitemaps`.
///
/// # Errors
///
/// The text for clap.
pub fn parse_page_url(value: &str) -> Result<String, String> {
    match Url::parse(value) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.host().is_some() => {
            Ok(value.to_owned())
        }
        _ => Err(format!("{value:?} is not an http or https URL")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_property_forms_are_normalized() {
        let ok = [
            ("sc-domain:Example.com", "sc-domain:example.com"),
            ("https://Example.com", "https://example.com/"),
            ("https://example.com/", "https://example.com/"),
            ("http://example.com/blog/", "http://example.com/blog/"),
            (" https://example.com/ ", "https://example.com/"),
        ];
        for (given, want) in ok {
            assert_eq!(Site::parse(given).unwrap().as_str(), want, "{given}");
        }
    }

    #[test]
    fn anything_else_names_both_forms() {
        for bad in [
            "example.com",
            "sc-domain:",
            "sc-domain:https://example.com",
            "ftp://example.com/",
            "localhost:8080",
            "",
        ] {
            let error = Site::parse(bad).unwrap_err();
            assert!(error.contains("sc-domain:example.com"), "{bad}: {error}");
        }
    }

    #[test]
    fn a_page_url_is_http_or_https_and_kept_as_given() {
        assert_eq!(
            parse_page_url("https://example.com/a?b=1").unwrap(),
            "https://example.com/a?b=1"
        );
        for bad in ["example.com/a", "mailto:a@b.c", "file:///etc/hosts"] {
            assert!(parse_page_url(bad).is_err(), "{bad}");
        }
    }
}
