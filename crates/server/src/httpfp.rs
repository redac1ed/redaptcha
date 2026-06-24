use axum::http::HeaderMap;

fn header<'a>(headers: &'a HeaderMap, k: &str) -> Option<&'a str> {
    headers.get(k).and_then(|v| v.to_str().ok())
}

const NON_BROWSER_AGENTS: [&str; 16] = [
    "python", "curl", "wget", "go-http", "java", "okhttp", "libwww", "urllib",
    "httpx", "requests", "node-fetch", "axios", "ruby", "perl", "scrapy", "httpclient",
];

pub fn is_disabled() -> bool {
    matches!(std::env::var("REDAPTCHA_HEADER_FP_OFF"), Ok(v) if v == "1" || v.eq_ignore_ascii_case("true"))
}

pub fn ja4_consistent(headers: &HeaderMap) -> Result<f64, &'static str> {
    let ja4 = match header(headers, "x-ja4") {
        Some(v) if !v.is_empty() => v,
        _ => return Ok(1.0),
    };
    let ua = header(headers, "user-agent").unwrap_or("").to_ascii_lowercase();
    if ja4.len() < 5 {
        return Err("malformed ja4");
    }
    let bytes = ja4.as_bytes();
    if bytes[0] != b't' && bytes[0] != b'q' {
        return Err("non-tls transport");
    }
    let ver = &ja4[1..3];
    let is_modern_browser = ua.contains("chrome")
        || ua.contains("firefox")
        || ua.contains("safari")
        || ua.contains("edg/");
    if is_modern_browser && ver != "13" {
        return Err("tls version vs ua mismatch");
    }
    if is_modern_browser && ja4.len() >= 10 {
        let alpn = &ja4[8..10];
        if alpn != "h2" {
            return Err("alpn vs ua mismatch");
        }
    }
    Ok(1.0)
}

pub fn browser_fingerprint(headers: &HeaderMap) -> Result<f64, &'static str> {
    let ua = header(headers, "user-agent").unwrap_or("").trim();
    if ua.is_empty() {
        return Err("missing user-agent");
    }
    let ua_l = ua.to_ascii_lowercase();
    if NON_BROWSER_AGENTS.iter().any(|s| ua_l.contains(s)) {
        return Err("non-browser user-agent");
    }
    if !ua_l.contains("mozilla/") {
        return Err("user-agent not a browser");
    }
    let has_sec_fetch = header(headers, "sec-fetch-mode").is_some()
        || header(headers, "sec-fetch-dest").is_some()
        || header(headers, "sec-fetch-site").is_some();
    if !has_sec_fetch {
        return Err("missing sec-fetch headers");
    }
    let is_chromium =
        ua_l.contains("chrome") || ua_l.contains("chromium") || ua_l.contains("edg/");
    if is_chromium && header(headers, "sec-ch-ua").is_none() {
        return Err("chromium ua without client hints");
    }
    let mut score = 1.0_f64;
    if header(headers, "accept-language").is_none() {
        score *= 0.6;
    }
    match header(headers, "accept-encoding") {
        Some(ae) if ae.contains("gzip") || ae.contains("br") => {}
        _ => score *= 0.6,
    }
    if header(headers, "accept").is_none() {
        score *= 0.7;
    }
    if let Some(m) = header(headers, "sec-fetch-mode") {
        if m != "cors" && m != "no-cors" && m != "navigate" {
            score *= 0.7;
        }
    }
    if is_chromium {
        match header(headers, "sec-ch-ua-mobile") {
            Some(_) => {}
            None => score *= 0.8,
        }
    }
    Ok(score.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for (k, v) in pairs {
            m.insert(
                axum::http::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        m
    }

    #[test]
    fn rejects_python_urllib() {
        let m = hm(&[("user-agent", "Python-urllib/3.13")]);
        assert!(browser_fingerprint(&m).is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(browser_fingerprint(&HeaderMap::new()).is_err());
    }

    #[test]
    fn rejects_spoofed_chrome_ua_without_headers() {
        let m = hm(&[(
            "user-agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        )]);
        assert!(browser_fingerprint(&m).is_err());
    }

    #[test]
    fn rejects_chromium_without_client_hints() {
        let m = hm(&[
            ("user-agent", "Mozilla/5.0 Chrome/120.0 Safari/537.36"),
            ("sec-fetch-mode", "cors"),
        ]);
        assert!(browser_fingerprint(&m).is_err());
    }

    #[test]
    fn accepts_real_chrome_fetch() {
        let m = hm(&[
            (
                "user-agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
            ),
            ("accept", "*/*"),
            ("accept-language", "en-US,en;q=0.9"),
            ("accept-encoding", "gzip, deflate, br"),
            ("sec-fetch-mode", "cors"),
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-dest", "empty"),
            ("sec-ch-ua", "\"Chromium\";v=\"120\", \"Not=A?Brand\";v=\"24\""),
            ("sec-ch-ua-mobile", "?0"),
        ]);
        let s = browser_fingerprint(&m).unwrap();
        assert!(s > 0.9, "score was {s}");
    }

    #[test]
    fn accepts_real_firefox_fetch() {
        let m = hm(&[
            (
                "user-agent",
                "Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0",
            ),
            ("accept", "*/*"),
            ("accept-language", "en-US,en;q=0.5"),
            ("accept-encoding", "gzip, deflate, br"),
            ("sec-fetch-mode", "cors"),
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-dest", "empty"),
        ]);
        assert!(browser_fingerprint(&m).is_ok());
    }

    #[test]
    fn ja4_absent_is_neutral() {
        assert_eq!(ja4_consistent(&HeaderMap::new()).unwrap(), 1.0);
    }

    #[test]
    fn ja4_python_tls12_with_chrome_ua_rejected() {
        let m = hm(&[
            ("x-ja4", "t12d1307h1_c16a28f6ef30_000000000000"),
            (
                "user-agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0 Safari/537.36",
            ),
        ]);
        assert!(ja4_consistent(&m).is_err());
    }

    #[test]
    fn ja4_chrome_tls13_h2_accepted() {
        let m = hm(&[
            ("x-ja4", "t13d1516h2_8daaf6152771_b0da82dd1658"),
            (
                "user-agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0 Safari/537.36",
            ),
        ]);
        assert_eq!(ja4_consistent(&m).unwrap(), 1.0);
    }

    #[test]
    fn ja4_non_tls_transport_rejected() {
        let m = hm(&[("x-ja4", "x99d0000h1_000000000000_000000000000")]);
        assert!(ja4_consistent(&m).is_err());
    }

    #[test]
    fn ja4_browser_ua_without_h2_alpn_rejected() {
        let m = hm(&[
            ("x-ja4", "t13d1312000000_aaaaaaaaaaaa_bbbbbbbbbbbb"),
            (
                "user-agent",
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0 Safari/537.36",
            ),
        ]);
        assert!(ja4_consistent(&m).is_err());
    }
}
