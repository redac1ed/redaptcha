use std::collections::HashMap;

#[derive(Clone)]
pub struct SiteKey {
    pub secret: String,
    pub hosts: Vec<String>,
}

#[derive(Clone, Default)]
pub struct SiteRegistry {
    keys: HashMap<String, SiteKey>
}

impl SiteRegistry {
    pub fn from_env() -> Self { Self::parse(&std::env::var("REDAPTCHA_SITES").unwrap_or_default()) }
    pub fn parse(raw: &str) -> Self {
        let mut keys = HashMap::new();
        for entry in raw.split(',') {
            let entry = entry.trim();
            if entry.is_empty() { continue; }
            let mut parts = entry.split(':');
            let site_key = parts.next().unwrap_or("").trim();
            let secret = parts.next().unwrap_or("").trim();
            let hosts_raw = parts.next().unwrap_or("").trim();
            if site_key.is_empty() || secret.is_empty() { continue; }
            let hosts = hosts_raw.split('|').map(str::trim)
                .filter(|h| !h.is_empty()).map(|h| h.to_ascii_lowercase()).collect();
            keys.insert(site_key.to_string(), SiteKey { secret: secret.to_string(), hosts });
        }
        SiteRegistry { keys }
    }
    pub fn is_empty(&self) -> bool { self.keys.is_empty() }
    pub fn len(&self) -> usize { self.keys.len() }
    pub fn contains(&self, k: &str) -> bool { self.keys.contains_key(k) }
    pub fn host_allowed(&self, k: &str, host: &str) -> bool {
        match self.keys.get(k) {
            None => false,
            Some(v) => v.hosts.is_empty() || v.hosts.iter().any(|a| a == &host.trim().to_ascii_lowercase()),
        }
    }
    pub fn secret_matches(&self, k: &str, secret: &str) -> bool {
        self.keys.get(k).map_or(false, |v| v.secret == secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_registry_is_open() {
        assert!(SiteRegistry::parse("").is_empty());
    }
    #[test]
    fn parses_key_secret_hosts() {
        let r = SiteRegistry::parse("rk_a:sec1:example.com|shop.example.com, rk_b:sec2:");
        assert!(r.contains("rk_a"));
        assert!(r.contains("rk_b"));
        assert!(r.secret_matches("rk_a", "sec1"));
        assert!(!r.secret_matches("rk_a", "wrong"));
        assert!(r.host_allowed("rk_a", "shop.example.com"));
        assert!(!r.host_allowed("rk_a", "evil.com"));
        assert!(r.host_allowed("rk_b", "anything.com"));
    }
    #[test]
    fn unknown_key_rejected() {
        let r = SiteRegistry::parse("rk_a:sec1:");
        assert!(!r.contains("rk_x"));
        assert!(!r.host_allowed("rk_x", "example.com"));
        assert!(!r.secret_matches("rk_x", "sec1"));
    }
}
