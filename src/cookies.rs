//! Browser cookie bridge and Netscape matching. Never broaden browser scope.
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserCookie {
    pub domain: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub secure: bool,
    pub host_only: bool,
    pub http_only: bool,
    pub session: bool,
    pub expiration_date: Option<f64>,
}

fn domain_matches(host: &str, domain: &str, subdomains: bool) -> bool {
    host == domain || (subdomains && host.ends_with(&format!(".{domain}")))
}

fn path_matches(request: &str, cookie: &str) -> bool {
    request == cookie
        || (request.starts_with(cookie)
            && (cookie.ends_with('/') || request.as_bytes().get(cookie.len()) == Some(&b'/')))
}

pub(crate) fn netscape_for(
    url: &str,
    cookies: &[BrowserCookie],
    now: i64,
) -> Result<String, String> {
    let url = Url::parse(url).map_err(|_| "Invalid cookie URL")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Cookie URL must be HTTP(S)".into());
    }
    let host = url.host_str().ok_or("Missing cookie host")?;
    if cookies.len() > 1024 {
        return Err("Too many cookies".into());
    }
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies {
        let domain = c.domain.trim_start_matches('.').to_ascii_lowercase();
        if domain.is_empty()
            || !domain_matches(host, &domain, !c.host_only)
            || !c.path.starts_with('/')
            || !path_matches(url.path(), &c.path)
            || (c.secure && url.scheme() != "https")
        {
            continue;
        }
        if [&c.domain, &c.name, &c.value, &c.path]
            .iter()
            .any(|s| s.chars().any(char::is_control))
            || c.name.is_empty()
            || c.name.contains([';', '='])
            || c.value.contains(';')
        {
            continue;
        }
        let expires = if c.session {
            0
        } else {
            let Some(expiry) = c
                .expiration_date
                .filter(|n| n.is_finite() && *n > now as f64)
            else {
                continue;
            };
            expiry as i64
        };
        let prefix = if c.http_only { "#HttpOnly_" } else { "" };
        let domain_prefix = if c.host_only { "" } else { "." };
        out.push_str(&format!(
            "{prefix}{domain_prefix}{domain}\t{}\t{}\t{}\t{expires}\t{}\t{}\n",
            if c.host_only { "FALSE" } else { "TRUE" },
            c.path,
            if c.secure { "TRUE" } else { "FALSE" },
            c.name,
            c.value
        ));
        if out.len() > 256 * 1024 {
            return Err("Cookies too large".into());
        }
    }
    Ok(out)
}

pub(crate) fn header_for(url: &str, text: &str, now: i64) -> Option<String> {
    let url = Url::parse(url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<_> = line.split('\t').collect();
        if f.len() != 7 || !matches!(f[1], "TRUE" | "FALSE") || !matches!(f[3], "TRUE" | "FALSE") {
            continue;
        }
        let domain = f[0].trim_start_matches('.').to_ascii_lowercase();
        let Ok(expiry) = f[4].parse::<i64>() else {
            continue;
        };
        if domain.is_empty()
            || !domain_matches(&host, &domain, f[1] == "TRUE")
            || !f[2].starts_with('/')
            || !path_matches(url.path(), f[2])
            || (f[3] == "TRUE" && url.scheme() != "https")
            || (expiry != 0 && expiry <= now)
            || f[5].is_empty()
            || f[5].contains([';', '='])
            || f[6].contains(';')
            || f.iter().any(|v| v.chars().any(char::is_control))
        {
            continue;
        }
        pairs.push(format!("{}={}", f[5], f[6]));
    }
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookie_attributes_bound_header_scope() {
        let text = "#HttpOnly_example.com\tFALSE\t/private\tTRUE\t200\tsid\tsecret\n";
        assert_eq!(
            header_for("https://example.com/private/file", text, 100).as_deref(),
            Some("sid=secret")
        );
        for url in [
            "http://example.com/private",
            "https://sub.example.com/private",
            "https://example.com/privately",
            "https://evil.test/private",
        ] {
            assert!(header_for(url, text, 100).is_none(), "{url}");
        }
        assert!(header_for("https://example.com/private", text, 200).is_none());
        let shared = ".example.com\tTRUE\t/\tFALSE\t0\ta\tb\n";
        assert!(header_for("https://sub.example.com/file", shared, 500).is_some());
        assert!(header_for("https://notexample.com/file", shared, 500).is_none());
    }
    #[test]
    fn export_preserves_attributes_and_rejects_injection() {
        let mut c = BrowserCookie {
            domain: "example.com".into(),
            name: "sid".into(),
            value: "secret".into(),
            path: "/private".into(),
            secure: true,
            host_only: true,
            http_only: true,
            session: false,
            expiration_date: Some(200.0),
        };
        let text = netscape_for(
            "https://example.com/private/file",
            std::slice::from_ref(&c),
            100,
        )
        .unwrap();
        assert!(text.contains("#HttpOnly_example.com\tFALSE\t/private\tTRUE\t200\tsid\tsecret"));
        c.value = "x\nmalicious".into();
        assert!(!netscape_for("https://example.com/private", &[c], 100)
            .unwrap()
            .contains("malicious"));
    }
}
