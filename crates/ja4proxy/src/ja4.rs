use sha2::{Digest, Sha256};

const GREASE: [u16; 16] = [
    0x0a0a, 0x1a1a, 0x2a2a, 0x3a3a, 0x4a4a, 0x5a5a, 0x6a6a, 0x7a7a, 0x8a8a, 0x9a9a, 0xaaaa, 0xbaba,
    0xcaca, 0xdada, 0xeaea, 0xfafa,
];

fn is_grease(v: u16) -> bool {
    GREASE.contains(&v)
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        let hi = self.u8()? as u16;
        let lo = self.u8()? as u16;
        Some((hi << 8) | lo)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let s = self.b.get(self.pos..end)?;
        self.pos = end;
        Some(s)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        let end = self.pos.checked_add(n)?;
        if end > self.b.len() {
            return None;
        }
        self.pos = end;
        Some(())
    }
}

pub struct ClientHello {
    pub tls_version: u16,
    pub ciphers: Vec<u16>,
    pub extensions: Vec<u16>,
    pub sni: bool,
    pub alpn: Vec<String>,
    pub sig_algs: Vec<u16>,
    pub supported_versions: Vec<u16>,
}

pub fn parse_client_hello(record: &[u8]) -> Option<ClientHello> {
    let mut r = Reader::new(record);
    let content_type = r.u8()?;
    if content_type != 0x16 {
        return None;
    }
    let _ver = r.u16()?;
    let rec_len = r.u16()? as usize;
    let body = r.take(rec_len.min(record.len().saturating_sub(5)))?;
    let mut h = Reader::new(body);
    let hs_type = h.u8()?;
    if hs_type != 0x01 {
        return None;
    }
    let _len = {
        let a = h.u8()? as usize;
        let b = h.u8()? as usize;
        let c = h.u8()? as usize;
        (a << 16) | (b << 8) | c
    };
    let legacy_version = h.u16()?;
    h.skip(32)?;
    let sid_len = h.u8()? as usize;
    h.skip(sid_len)?;
    let cs_len = h.u16()? as usize;
    let cs_bytes = h.take(cs_len)?;
    let mut ciphers = Vec::new();
    let mut cr = Reader::new(cs_bytes);
    while let Some(c) = cr.u16() {
        if !is_grease(c) {
            ciphers.push(c);
        }
    }
    let comp_len = h.u8()? as usize;
    h.skip(comp_len)?;

    let mut extensions = Vec::new();
    let mut sni = false;
    let mut alpn = Vec::new();
    let mut sig_algs = Vec::new();
    let mut supported_versions = Vec::new();

    if let Some(ext_total) = h.u16() {
        let ext_bytes = h.take(ext_total as usize)?;
        let mut er = Reader::new(ext_bytes);
        while let (Some(etype), Some(elen)) = (er.u16(), er.u16()) {
            let edata = er.take(elen as usize)?;
            if is_grease(etype) {
                continue;
            }
            extensions.push(etype);
            match etype {
                0x0000 => sni = true,
                0x0010 => {
                    let mut ar = Reader::new(edata);
                    if let Some(list_len) = ar.u16() {
                        let list = ar.take(list_len as usize).unwrap_or(&[]);
                        let mut lr = Reader::new(list);
                        while let Some(plen) = lr.u8() {
                            if let Some(p) = lr.take(plen as usize) {
                                if let Ok(s) = std::str::from_utf8(p) {
                                    alpn.push(s.to_string());
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
                0x000d => {
                    let mut sr = Reader::new(edata);
                    if let Some(l) = sr.u16() {
                        let list = sr.take(l as usize).unwrap_or(&[]);
                        let mut lr = Reader::new(list);
                        while let Some(v) = lr.u16() {
                            if !is_grease(v) {
                                sig_algs.push(v);
                            }
                        }
                    }
                }
                0x002b => {
                    let mut vr = Reader::new(edata);
                    if let Some(l) = vr.u8() {
                        let list = vr.take(l as usize).unwrap_or(&[]);
                        let mut lr = Reader::new(list);
                        while let Some(v) = lr.u16() {
                            if !is_grease(v) {
                                supported_versions.push(v);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let tls_version = supported_versions
        .iter()
        .copied()
        .filter(|v| *v >= 0x0301 && *v <= 0x0304)
        .max()
        .unwrap_or(legacy_version);

    Some(ClientHello {
        tls_version,
        ciphers,
        extensions,
        sni,
        alpn,
        sig_algs,
        supported_versions,
    })
}

fn ver_str(v: u16) -> &'static str {
    match v {
        0x0304 => "13",
        0x0303 => "12",
        0x0302 => "11",
        0x0301 => "10",
        _ => "00",
    }
}

fn sha12_raw(s: &str) -> String {
    if s.is_empty() {
        return "000000000000".to_string();
    }
    let digest = Sha256::digest(s.as_bytes());
    hex::encode(&digest[..6])
}

pub fn ja4(ch: &ClientHello) -> String {
    let proto = "t";
    let ver = ver_str(ch.tls_version);
    let sni = if ch.sni { "d" } else { "i" };
    let nc = ch.ciphers.len().min(99);
    let ne = ch.extensions.len().min(99);
    let alpn_code = match ch.alpn.first() {
        Some(a) if a == "h2" => "h2".to_string(),
        Some(a) if a == "http/1.1" => "h1".to_string(),
        Some(a) if a.len() >= 2 => {
            let bytes = a.as_bytes();
            format!("{}{}", bytes[0] as char, bytes[bytes.len() - 1] as char)
        }
        _ => "00".to_string(),
    };

    let mut ciphers: Vec<u16> = ch.ciphers.clone();
    ciphers.sort_unstable();
    let cipher_joined = ciphers
        .iter()
        .map(|c| format!("{:04x}", c))
        .collect::<Vec<_>>()
        .join(",");

    let mut exts: Vec<u16> = ch
        .extensions
        .iter()
        .copied()
        .filter(|e| *e != 0x0000 && *e != 0x0010)
        .collect();
    exts.sort_unstable();
    let ext_joined = exts
        .iter()
        .map(|e| format!("{:04x}", e))
        .collect::<Vec<_>>()
        .join(",");

    let ext_hash_input = if ch.sig_algs.is_empty() {
        ext_joined
    } else {
        let sig_joined = ch
            .sig_algs
            .iter()
            .map(|s| format!("{:04x}", s))
            .collect::<Vec<_>>()
            .join(",");
        format!("{}_{}", ext_joined, sig_joined)
    };

    let cipher_hash = sha12_raw(&cipher_joined);
    let ext_hash = sha12_raw(&ext_hash_input);
    format!(
        "{}{}{}{:02}{:02}{}_{}_{}",
        proto, ver, sni, nc, ne, alpn_code, cipher_hash, ext_hash
    )
}

pub fn fingerprint(record: &[u8]) -> Option<String> {
    parse_client_hello(record).map(|ch| ja4(&ch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chrome_hello() -> Vec<u8> {
        let mut body: Vec<u8> = Vec::new();
        body.push(0x01);
        let mut hs: Vec<u8> = Vec::new();
        hs.extend_from_slice(&[0x03, 0x03]);
        hs.extend_from_slice(&[0u8; 32]);
        hs.push(0x00);
        let ciphers: [u16; 3] = [0x1301, 0x1302, 0x1303];
        hs.extend_from_slice(&((ciphers.len() * 2) as u16).to_be_bytes());
        for c in ciphers {
            hs.extend_from_slice(&c.to_be_bytes());
        }
        hs.push(0x01);
        hs.push(0x00);
        let mut exts: Vec<u8> = Vec::new();
        let push_ext = |exts: &mut Vec<u8>, t: u16, data: &[u8]| {
            exts.extend_from_slice(&t.to_be_bytes());
            exts.extend_from_slice(&(data.len() as u16).to_be_bytes());
            exts.extend_from_slice(data);
        };
        push_ext(&mut exts, 0x0000, &[0x00, 0x05, 0x00, 0x00, 0x02, b'h', b'i']);
        let alpn = [0x00u8, 0x03, 0x02, b'h', b'2'];
        push_ext(&mut exts, 0x0010, &alpn);
        let sv = [0x02u8, 0x03, 0x04];
        push_ext(&mut exts, 0x002b, &sv);
        push_ext(&mut exts, 0x000a, &[0x00, 0x02, 0x00, 0x1d]);
        hs.extend_from_slice(&(exts.len() as u16).to_be_bytes());
        hs.extend_from_slice(&exts);
        let hs_len = hs.len();
        body.push(((hs_len >> 16) & 0xff) as u8);
        body.push(((hs_len >> 8) & 0xff) as u8);
        body.push((hs_len & 0xff) as u8);
        body.extend_from_slice(&hs);
        let mut record: Vec<u8> = Vec::new();
        record.push(0x16);
        record.extend_from_slice(&[0x03, 0x01]);
        record.extend_from_slice(&(body.len() as u16).to_be_bytes());
        record.extend_from_slice(&body);
        record
    }

    #[test]
    fn parses_and_fingerprints_chrome_like_hello() {
        let rec = sample_chrome_hello();
        let ch = parse_client_hello(&rec).expect("parse");
        assert_eq!(ch.tls_version, 0x0304);
        assert!(ch.sni);
        assert_eq!(ch.alpn.first().map(String::as_str), Some("h2"));
        assert_eq!(ch.ciphers, vec![0x1301, 0x1302, 0x1303]);
        let fp = ja4(&ch);
        assert!(fp.starts_with("t13d"), "fp = {fp}");
        assert!(fp.contains("h2"), "fp = {fp}");
    }

    #[test]
    fn fingerprint_is_stable() {
        let rec = sample_chrome_hello();
        assert_eq!(fingerprint(&rec), fingerprint(&rec));
    }

    #[test]
    fn rejects_non_handshake() {
        assert!(parse_client_hello(&[0x17, 0x03, 0x03, 0x00, 0x00]).is_none());
    }

    #[test]
    fn extension_order_does_not_change_fingerprint() {
        let rec = sample_chrome_hello();
        let ch1 = parse_client_hello(&rec).unwrap();
        let mut ch2 = parse_client_hello(&rec).unwrap();
        ch2.extensions.reverse();
        assert_eq!(ja4(&ch1), ja4(&ch2));
    }
}
