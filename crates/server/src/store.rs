use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

const REDEEM_TTL_SECS: u64 = 600;
const MAX_RESP_BULK: usize = 1 << 20;

pub struct RawRedis {
    host: String,
    port: u16,
    username: String,
    password: String,
    tls: bool,
}

pub enum RedeemStore {
    Memory(Mutex<HashMap<String, Instant>>),
    Redis(RawRedis),
}

trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

fn tls_connector() -> TlsConnector {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

impl RawRedis {
    fn from_url(url: &str) -> Option<Self> {
        let trimmed = url.trim();
        let (tls, rest) = if let Some(r) = trimmed.strip_prefix("rediss://") {
            (true, r)
        } else if let Some(r) = trimmed.strip_prefix("redis://") {
            (false, r)
        } else {
            return None;
        };
        let (auth, hostport) = match rest.rfind('@') {
            Some(i) => (&rest[..i], &rest[i + 1..]),
            None => ("", rest),
        };
        let hostport = hostport.split('/').next().unwrap_or(hostport);
        let (username, password) = match auth.find(':') {
            Some(i) => (&auth[..i], &auth[i + 1..]),
            None => ("default", auth),
        };
        let (host, port_str) = match hostport.rfind(':') {
            Some(i) => (&hostport[..i], &hostport[i + 1..]),
            None => (hostport, "6379"),
        };
        Some(Self {
            host: host.to_string(),
            port: port_str.parse().ok()?,
            username: if username.is_empty() { "default".into() } else { username.to_string() },
            password: password.to_string(),
            tls,
        })
    }

    pub fn is_tls(&self) -> bool {
        self.tls
    }

    async fn open(&self) -> Result<Box<dyn AsyncStream>, String> {
        let tcp = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .map_err(|e| format!("connect: {e}"))?;
        if self.tls {
            let name = ServerName::try_from(self.host.clone())
                .map_err(|e| format!("tls servername: {e}"))?;
            let stream = tls_connector()
                .connect(name, tcp)
                .await
                .map_err(|e| format!("tls handshake: {e}"))?;
            Ok(Box::new(stream))
        } else {
            Ok(Box::new(tcp))
        }
    }

    pub async fn ping(&self) -> Result<(), String> {
        let stream = self.open().await?;
        let (rd, mut wr) = tokio::io::split(stream);
        let mut buf = BufReader::new(rd);
        wr.write_all(&resp_cmd(&["AUTH", &self.username, &self.password])).await.map_err(|e| e.to_string())?;
        wr.write_all(&resp_cmd(&["PING"])).await.map_err(|e| e.to_string())?;
        wr.flush().await.map_err(|e| e.to_string())?;
        read_resp(&mut buf).await?;
        read_resp(&mut buf).await?;
        Ok(())
    }

    pub async fn set_nx_ex(&self, key: &str, ttl: u64) -> Result<bool, String> {
        let stream = self.open().await?;
        let (rd, mut wr) = tokio::io::split(stream);
        let mut buf = BufReader::new(rd);
        let ttl_s = ttl.to_string();
        wr.write_all(&resp_cmd(&["AUTH", &self.username, &self.password])).await.map_err(|e| e.to_string())?;
        wr.write_all(&resp_cmd(&["SET", key, "1", "NX", "EX", &ttl_s])).await.map_err(|e| e.to_string())?;
        wr.flush().await.map_err(|e| e.to_string())?;
        read_resp(&mut buf).await?;
        let resp = read_resp(&mut buf).await?;
        Ok(resp.starts_with(b"+"))
    }
}

fn resp_cmd(parts: &[&str]) -> Vec<u8> {
    let mut v = format!("*{}\r\n", parts.len()).into_bytes();
    for p in parts {
        v.extend_from_slice(format!("${}\r\n{}\r\n", p.len(), p).as_bytes());
    }
    v
}

async fn read_resp(r: &mut BufReader<impl AsyncReadExt + Unpin>) -> Result<Vec<u8>, String> {
    let mut line = Vec::new();
    r.read_until(b'\n', &mut line).await.map_err(|e| e.to_string())?;
    if line.is_empty() {
        return Err("empty response from redis".into());
    }
    match line[0] {
        b'+' | b':' => Ok(line),
        b'-' => Err(format!("redis: {}", String::from_utf8_lossy(&line[1..]).trim())),
        b'$' => {
            let s = String::from_utf8_lossy(&line[1..]).trim_end_matches("\r\n").to_string();
            let n: i64 = s.parse().map_err(|_| format!("bad bulk len: {s}"))?;
            if n < 0 {
                return Ok(b"nil".to_vec());
            }
            if n as usize > MAX_RESP_BULK {
                return Err(format!("redis bulk too large: {n}"));
            }
            let mut data = vec![0u8; n as usize + 2];
            r.read_exact(&mut data).await.map_err(|e| e.to_string())?;
            data.truncate(n as usize);
            Ok(data)
        }
        b => Err(format!("unexpected resp prefix: {b}")),
    }
}

fn is_production() -> bool {
    matches!(
        std::env::var("REDAPTCHA_ENV").unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "production" | "prod"
    )
}

impl RedeemStore {
    pub async fn from_env() -> Self {
        match std::env::var("REDIS_URL") {
            Ok(url) if !url.trim().is_empty() => {
                match RawRedis::from_url(url.trim()) {
                    Some(r) => {
                        if is_production() && !r.is_tls() {
                            eprintln!("warning: REDIS_URL uses plaintext redis:// in production; switch to rediss:// for TLS");
                        }
                        match r.ping().await {
                            Ok(_) => {
                                println!("redeem log: using Redis");
                                RedeemStore::Redis(r)
                            }
                            Err(e) => {
                                if is_production() {
                                    panic!("REDIS_URL set but connect failed in production ({e}); refusing in-memory fallback");
                                }
                                eprintln!("warning: REDIS_URL set but connect failed ({e}); using in-memory redeem log");
                                RedeemStore::Memory(Mutex::new(HashMap::new()))
                            }
                        }
                    }
                    None => {
                        if is_production() {
                            panic!("REDIS_URL set but could not be parsed in production; refusing in-memory fallback");
                        }
                        eprintln!("warning: REDIS_URL could not be parsed; using in-memory redeem log");
                        RedeemStore::Memory(Mutex::new(HashMap::new()))
                    }
                }
            }
            _ => {
                if is_production() {
                    panic!("REDIS_URL not set in production; in-memory redeem log allows token replay across restarts");
                }
                println!("redeem log: in-memory (set REDIS_URL to share across dynos/restarts)");
                RedeemStore::Memory(Mutex::new(HashMap::new()))
            }
        }
    }

    pub async fn try_consume(&self, token: &str) -> bool {
        match self {
            RedeemStore::Memory(m) => {
                let mut seen = m.lock().await;
                let cutoff = std::time::Duration::from_secs(REDEEM_TTL_SECS);
                seen.retain(|_, t| t.elapsed() < cutoff);
                if seen.contains_key(token) {
                    return false;
                }
                seen.insert(token.to_string(), Instant::now());
                true
            }
            RedeemStore::Redis(r) => {
                let key = format!("redeem:{token}");
                match r.set_nx_ex(&key, REDEEM_TTL_SECS).await {
                    Ok(set) => set,
                    Err(e) => {
                        eprintln!("redis redeem error ({e}); failing closed");
                        false
                    }
                }
            }
        }
    }
}