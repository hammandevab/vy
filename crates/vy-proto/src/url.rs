//! The little the fetcher needs of HTTP URLs and headers: splitting an
//! absolute `http(s)://` URL, resolving a reference against it (RFC 3986
//! §5 for the forms a server can send), and reading `Content-Type` and
//! `Cache-Control`.

/// Parts of an absolute HTTP URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpUrl {
    /// `http` or `https`.
    pub scheme: String,
    /// Host as written (lower-cased).
    pub host: String,
    /// Explicit port.
    pub port: Option<u16>,
    /// Path with query, starting with `/`.
    pub path_and_query: String,
}

impl HttpUrl {
    /// Split an absolute `http://` or `https://` URL; `None` otherwise.
    pub fn parse(url: &str) -> Option<HttpUrl> {
        let (scheme, rest) = url.split_once("://")?;
        if scheme != "http" && scheme != "https" {
            return None;
        }
        let (authority, path_and_query) = match rest.find(['/', '?']) {
            Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
            Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() || authority.contains('@') {
            return None;
        }
        let (host, port) = if authority.starts_with('[') {
            let end = authority.find(']')?;
            let port = match &authority[end + 1..] {
                "" => None,
                p => Some(p.strip_prefix(':')?.parse().ok()?),
            };
            (authority[..=end].to_ascii_lowercase(), port)
        } else {
            match authority.split_once(':') {
                Some((h, p)) => (h.to_ascii_lowercase(), Some(p.parse().ok()?)),
                None => (authority.to_ascii_lowercase(), None),
            }
        };
        if host.is_empty() || path_and_query.contains('#') {
            return None;
        }
        Some(HttpUrl {
            scheme: scheme.to_string(),
            host,
            port,
            path_and_query,
        })
    }

    /// True for the loopback hosts exempt from TLS (SPEC-7.1.3).
    pub fn is_localhost(&self) -> bool {
        matches!(self.host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
    }

    /// `scheme://host[:port]`.
    pub fn origin(&self) -> String {
        match self.port {
            Some(p) => format!("{}://{}:{p}", self.scheme, self.host),
            None => format!("{}://{}", self.scheme, self.host),
        }
    }

    /// The whole URL.
    pub fn to_string_full(&self) -> String {
        format!("{}{}", self.origin(), self.path_and_query)
    }
}

/// Resolve `reference` against the absolute URL `base`: absolute URLs
/// pass through, `//host/...` keeps the scheme, `/...` keeps the origin,
/// `?...` keeps the path, anything else replaces the last path segment.
/// Fragments are dropped. `None` when the result is not an HTTP URL.
pub fn resolve(base: &str, reference: &str) -> Option<String> {
    let reference = reference.split('#').next().unwrap_or("");
    if reference.contains("://") {
        return HttpUrl::parse(reference).map(|u| u.to_string_full());
    }
    let base = HttpUrl::parse(base)?;
    let joined = if let Some(rest) = reference.strip_prefix("//") {
        format!("{}://{rest}", base.scheme)
    } else if reference.starts_with('/') {
        format!("{}{reference}", base.origin())
    } else if reference.starts_with('?') {
        let path = base.path_and_query.split('?').next().unwrap_or("/");
        format!("{}{path}{reference}", base.origin())
    } else if reference.is_empty() {
        base.to_string_full()
    } else {
        let path = base.path_and_query.split('?').next().unwrap_or("/");
        let dir = &path[..=path.rfind('/').unwrap_or(0)];
        format!("{}{dir}{reference}", base.origin())
    };
    HttpUrl::parse(&joined).map(|u| u.to_string_full())
}

/// The media type of a `Content-Type` value: lower-case, parameters dropped.
pub fn media_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// What `Cache-Control` says about storing a response (SPEC-7.3.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheControl {
    /// `max-age` in seconds; 0 when absent.
    pub max_age: u64,
    /// `no-store` or `no-cache`: do not reuse without revalidation.
    pub no_store: bool,
}

/// Parse a `Cache-Control` header value.
pub fn cache_control(value: &str) -> CacheControl {
    let mut cc = CacheControl::default();
    for directive in value.split(',') {
        let d = directive.trim().to_ascii_lowercase();
        if d == "no-store" || d == "no-cache" {
            cc.no_store = true;
        } else if let Some(v) = d.strip_prefix("max-age=") {
            cc.max_age = v.trim_matches('"').parse().unwrap_or(0);
        }
    }
    cc
}

/// Whether a media range list (an `Accept` header) admits `media`.
pub fn accepts(accept: &str, media: &str) -> bool {
    accept.split(',').any(|range| {
        let r = range
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        r == media
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_resolve() {
        let u = HttpUrl::parse("https://Ex.se:8443/a/b?x=1").unwrap();
        assert_eq!(u.origin(), "https://ex.se:8443");
        assert_eq!(u.path_and_query, "/a/b?x=1");
        assert!(HttpUrl::parse("ftp://x/").is_none());
        assert!(HttpUrl::parse("https://u@x/").is_none());
        let base = "https://ex.se/dir/page?q=1";
        assert_eq!(resolve(base, "/_vy/abc").unwrap(), "https://ex.se/_vy/abc");
        assert_eq!(resolve(base, "blob").unwrap(), "https://ex.se/dir/blob");
        assert_eq!(resolve(base, "?z").unwrap(), "https://ex.se/dir/page?z");
        assert_eq!(
            resolve(base, "//cdn.example/x").unwrap(),
            "https://cdn.example/x"
        );
        assert_eq!(
            resolve(base, "http://localhost:1/x#frag").unwrap(),
            "http://localhost:1/x"
        );
        assert_eq!(resolve(base, "").unwrap(), base);
        assert!(resolve(base, "ftp://x/y").is_none());
        assert!(HttpUrl::parse("http://localhost:8080/")
            .unwrap()
            .is_localhost());
    }

    #[test]
    fn headers() {
        assert_eq!(
            media_type("Application/VY-Package+zstd; charset=x"),
            "application/vy-package+zstd"
        );
        let cc = cache_control("public, max-age=60, immutable");
        assert_eq!(
            cc,
            CacheControl {
                max_age: 60,
                no_store: false
            }
        );
        assert!(cache_control("no-store").no_store);
        assert_eq!(cache_control("").max_age, 0);
        assert!(accepts(
            "application/vy-package+zstd, application/vy-manifest+cbor",
            "application/vy-manifest+cbor"
        ));
        assert!(!accepts(
            "text/html,*/*;q=0.8",
            "application/vy-manifest+cbor"
        ));
    }
}
