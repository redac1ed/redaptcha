use std::net::{Ipv4Addr, IpAddr};
use parking_lot::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MIN_SAFE_PREFIX: u32 = 8;
const MAX_RANGES: usize = 5_000_000;
const REFRESH_SECS: u64 = 6*3600;
const LIST_URL: &str = "http://az0-vpnip-public.oooninja.com/ip.txt";
const MAX_RESPONSE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpClass {
    Residential,
    Vpn,
    Unknown,
}

pub struct NetIntel {
    ranges:RwLock<Vec<(u32, u32)>>,
    enabled: bool, 
    block_unknown: bool,
}

impl NetIntel {
    pub fn from_env() -> Self {
        let enabled = truthy("REDAPTCHA_VPN_BLOCK");
        let block_unknown = truthy("REDAPTCHA_VPN_BLOCK_UNKNOWN");
        let mut ranges = Vec::new();
        if let Ok(path) = std::env::var("REDAPTCHA_BLOCK_CIDRS") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                load_lines(&text, &mut ranges);
            }
        }
        ranges.sort_by_key(|&(s, _)| s);
        NetIntel {
            ranges: RwLock::new(ranges),
            enabled,
            block_unknown,
        }
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
    pub fn classify(&self, ip: &str) -> IpClass {
        let v4 = match ip.parse::<IpAddr>() {
            Ok(IpAddr::V4(a)) => a,
            Ok(IpAddr::V6(a)) => match a.to_ipv4_mapped() {
                Some(m) => m,
                None => return IpClass::Unknown,
            },
            Err(_) => return IpClass::Unknown,
        };
        let n = u32::from(v4);
        if in_ranges(&self.ranges.read(), n) {
            IpClass::Vpn
        } else {
            IpClass::Residential
        }
    }
    pub fn should_block(&self, class: IpClass) -> bool {
        if !self.enabled {
            return false;
        }
        match class {
            IpClass::Vpn => true,
            IpClass::Unknown => self.block_unknown,
            IpClass::Residential => false,
        }
    }
    pub fn spawn_refresher(self: std::sync::Arc<Self>) {
        if !self.enabled {
            return;
        }
        tokio::spawn(async move {
            loop {
                if let Some(text) = fetch_http(LIST_URL).await {
                    let mut r = Vec::new();
                    load_lines(&text, &mut r);
                    if !r.is_empty() {
                        r.sort_by_key(|&(s, _)| s);
                        let n = r.len();
                        *self.ranges.write() = r;
                        eprintln!("netintel: loaded {n} VPN ranges");
                    }
                } else {
                    eprintln!("netintel: VPN list fetch failed, keeping previous");
                }
                tokio::time::sleep(std::time::Duration::from_secs(REFRESH_SECS)).await;
            }
        });
    }
}

fn truthy(var: &str) -> bool {
    matches!(
        std::env::var(var).unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn in_ranges(ranges: &[(u32, u32)], n:u32) -> bool {
    let mut lo = 0usize;
    let mut hi = ranges.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        let (s, e) = ranges[mid];
        if n < s {
            hi = mid;
        } else if n > e {
            lo = mid + 1;
        } else {
            return true;
        }
    }
    false
}

fn load_lines(text: &str, out: &mut Vec<(u32, u32)>) {
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some((s, e)) = parse_entry(line) {
            let span = (e - s) as u64 + 1;
            if span > (1u64 << (32 - MIN_SAFE_PREFIX)) {
                continue;
            }
            out.push((s, e));
            if out.len() >= MAX_RANGES {
                break;
            }
        }
    }
}

fn parse_entry(s: &str) -> Option<(u32, u32)> {
    let (addr, bits) = match s.split_once('/') {
        Some((a, b)) => (a, b.parse::<u32>().ok()?),
        None => (s, 32),
    };
    if bits > 32 {
        return None;
    }
    let base = u32::from(addr.trim().parse::<Ipv4Addr>().ok()?);
    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
    let start = base & mask;
    Some((start, start | !mask))
}

async fn fetch_http(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let (host_port, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(80)),
        None => (host_port.to_string(), 80u16),
    };
    let connect = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    .ok()?
    .ok()?;
    let mut stream = connect;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: redaptcha\r\nConnection: close\r\nAccept: text/plain\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await.ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let n = match tokio::time::timeout(
            std::time::Duration::from_secs(30),
            stream.read(&mut chunk),
        )
        .await
        {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => n,
            _ => return None,
        };
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_RESPONSE_BYTES {
            eprintln!("netintel: VPN list exceeded {MAX_RESPONSE_BYTES} bytes, aborting");
            return None;
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let status_ok = text
        .lines()
        .next()
        .map(|l| {
            let mut parts = l.split_whitespace();
            parts.next();
            parts.next() == Some("200")
        })
        .unwrap_or(false);
    if !status_ok {
        eprintln!("netintel: VPN list fetch non-200 status");
        return None;
    }
    let body = text.split("\r\n\r\n").nth(1)?;
    Some(strip_chunked(body))
}

fn strip_chunked(body: &str) -> String {
    let mut out = String::new();
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.len() <= 4 && u32::from_str_radix(t, 16).is_ok() {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}