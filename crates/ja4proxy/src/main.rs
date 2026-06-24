mod ja4;

use std::sync::Arc;
use std::fs::File;
use std::io::BufReader;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

const MAX_HELLO: usize = 16384;
const MAX_HEADERS: usize = 32768;

fn env_or(k: &str, d: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| d.to_string())
}

fn load_certs(path: &str) -> Vec<CertificateDer<'static>> {
    let f = File::open(path).unwrap_or_else(|e| panic!("cert {path}: {e}"));
    rustls_pemfile::certs(&mut BufReader::new(f))
        .filter_map(Result::ok)
        .collect()
}

fn load_key(path: &str) -> PrivateKeyDer<'static> {
    let f = File::open(path).unwrap_or_else(|e| panic!("key {path}: {e}"));
    rustls_pemfile::private_key(&mut BufReader::new(f))
        .ok()
        .flatten()
        .expect("no private key in key file")
}

async fn peek_ja4(stream: &TcpStream) -> Option<String> {
    let mut buf = vec![0u8; MAX_HELLO];
    for _ in 0..50 {
        let n = stream.peek(&mut buf).await.ok()?;
        if n < 6 {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            continue;
        }
        let rec_len = ((buf[3] as usize) << 8 | buf[4] as usize) + 5;
        if n >= rec_len || n >= MAX_HELLO {
            return ja4::fingerprint(&buf[..n]);
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    ja4::fingerprint(&buf)
}

fn sanitize(fp: &str) -> String {
    fp.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(64)
        .collect()
}

async fn read_headers<R: AsyncReadExt + Unpin>(r: &mut R) -> Option<Vec<u8>> {
    let mut buf = Vec::with_capacity(2048);
    let mut tmp = [0u8; 1024];
    loop {
        let n = r.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return Some(buf);
        }
        if buf.len() > MAX_HEADERS {
            return None;
        }
    }
}

fn inject_header(raw: &[u8], ja4: &str, peer_ip: &str) -> Option<Vec<u8>> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = &raw[..split];
    let body = &raw[split + 4..];
    let mut out = Vec::with_capacity(raw.len() + 80);
    for line in head.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let lower: Vec<u8> = line.iter().map(|b| b.to_ascii_lowercase()).collect();
        if lower.starts_with(b"x-ja4:")
            || lower.starts_with(b"x-forwarded-for:")
            || lower.starts_with(b"connection:")
            || lower.starts_with(b"keep-alive:")
        {
            continue;
        }
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("X-JA4: {}\r\n", ja4).as_bytes());
    out.extend_from_slice(format!("X-Forwarded-For: {}\r\n", peer_ip).as_bytes());
    out.extend_from_slice(b"Connection: close\r\n");
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(body);
    Some(out)
}

async fn handle(tcp: TcpStream, acceptor: TlsAcceptor, backend: String) {
    let peer_ip = tcp
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    let ja4 = peek_ja4(&tcp).await.map(|f| sanitize(&f)).unwrap_or_default();
    let tls = match acceptor.accept(tcp).await {
        Ok(s) => s,
        Err(_) => return,
    };
    let (mut cr, mut cw) = tokio::io::split(tls);
    let raw = match read_headers(&mut cr).await {
        Some(b) => b,
        None => return,
    };
    let rewritten = match inject_header(&raw, &ja4, &peer_ip) {
        Some(b) => b,
        None => return,
    };
    let backend_conn = match TcpStream::connect(&backend).await {
        Ok(s) => s,
        Err(_) => return,
    };
    let (mut br, mut bw) = backend_conn.into_split();
    if bw.write_all(&rewritten).await.is_err() {
        return;
    }
    let c2b = async {
        let _ = tokio::io::copy(&mut cr, &mut bw).await;
        let _ = bw.shutdown().await;
    };
    let b2c = async {
        let _ = tokio::io::copy(&mut br, &mut cw).await;
        let _ = cw.shutdown().await;
    };
    tokio::join!(c2b, b2c);
}

#[tokio::main]
async fn main() {
    let listen = env_or("JA4PROXY_LISTEN", "0.0.0.0:8443");
    let backend = env_or("JA4PROXY_BACKEND", "127.0.0.1:3000");
    let cert_path = env_or("JA4PROXY_CERT", "certs/cert.pem");
    let key_path = env_or("JA4PROXY_KEY", "certs/key.pem");

    let (certs, key) = if std::path::Path::new(&cert_path).exists()
        && std::path::Path::new(&key_path).exists()
    {
        (load_certs(&cert_path), load_key(&key_path))
    } else {
        println!("no cert/key found; generating ephemeral self-signed cert for localhost");
        let gen = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("self-signed gen");
        let cert = CertificateDer::from(gen.cert);
        let key = PrivateKeyDer::try_from(gen.key_pair.serialize_der()).expect("key der");
        (vec![cert], key)
    };
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("bad cert/key");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind(&listen).await.expect("bind");
    println!("ja4proxy listening on {listen} -> {backend}");
    loop {
        let (tcp, _addr) = match listener.accept().await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let _ = tcp.set_nodelay(true);
        let acceptor = acceptor.clone();
        let backend = backend.clone();
        tokio::spawn(handle(tcp, acceptor, backend));
    }
}
