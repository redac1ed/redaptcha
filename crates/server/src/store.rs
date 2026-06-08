use std::collections::HashMap;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

const REDEEM_TTL_SECS: u64 = 600;

pub struct RawRedis {
    host: String,
    port: u16,
    username: String,
    password: String,
}

pub enum RedeemStore {
    Memory(Mutex<HashMap<String, Instant>>),
    Redis(RawRedis),
}

impl RawRedis {
    fn from_url(url: &str) -> Option<Self> {
        let rest = url.trim().strip_prefix("redis://")?;
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
        })
    }

    async fn open(&self) -> Result<TcpStream, String> {
        TcpStream::connect((self.host.as_str(), self.port))
            .await
            .map_err(|e| format!("connect: {e}"))
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
            let mut data = vec![0u8; n as usize + 2];
            r.read_exact(&mut data).await.map_err(|e| e.to_string())?;
            data.truncate(n as usize);
            Ok(data)
        }
        b => Err(format!("unexpected resp prefix: {b}")),
    }
}

impl RedeemStore {
    pub async fn from_env() -> Self {
        match std::env::var("REDIS_URL") {
            Ok(url) if !url.trim().is_empty() => {
                match RawRedis::from_url(url.trim()) {
                    Some(r) => match r.ping().await {
                        Ok(_) => {
                            println!("redeem log: using Redis");
                            RedeemStore::Redis(r)
                        }
                        Err(e) => {
                            eprintln!("warning: REDIS_URL set but connect failed ({e}); using in-memory redeem log");
                            RedeemStore::Memory(Mutex::new(HashMap::new()))
                        }
                    },
                    None => {
                        eprintln!("warning: REDIS_URL could not be parsed; using in-memory redeem log");
                        RedeemStore::Memory(Mutex::new(HashMap::new()))
                    }
                }
            }
            _ => {
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
