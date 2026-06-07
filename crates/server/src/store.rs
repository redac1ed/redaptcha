use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::Mutex;

const REDEEM_TTL_SECS: u64 = 600;

pub enum RedeemStore {
    Memory(Mutex<HashMap<String, Instant>>),
    Redis(redis::Client),
}

impl RedeemStore {
    pub async fn from_env() -> Self {
        match std::env::var("REDIS_URL") {
            Ok(url) if !url.trim().is_empty() => match connect(&url).await {
                Ok(client) => {
                    println!("redeem log: using Redis");
                    RedeemStore::Redis(client)
                }
                Err(e) => {
                    eprintln!("warning: REDIS_URL set but connect failed ({e}); using in-memory redeem log");
                    RedeemStore::Memory(Mutex::new(HashMap::new()))
                }
            },
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
            RedeemStore::Redis(client) => {
                let mut c = match client.get_multiplexed_async_connection().await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("redis connect error ({e}); failing closed");
                        return false;
                    }
                };
                let key = format!("redeem:{token}");
                let set: redis::RedisResult<Option<String>> = redis::cmd("SET")
                    .arg(&key)
                    .arg(1)
                    .arg("NX")
                    .arg("EX")
                    .arg(REDEEM_TTL_SECS)
                    .query_async(&mut c)
                    .await;
                match set {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(e) => {
                        eprintln!("redis redeem error ({e}); failing closed");
                        false
                    }
                }
            }
        }
    }
}

async fn connect(url: &str) -> redis::RedisResult<redis::Client> {
    let mut info: redis::ConnectionInfo = url.parse()?;
    info.redis.protocol = redis::ProtocolVersion::RESP2;
    let client = redis::Client::open(info)?;
    let mut conn = client.get_multiplexed_async_connection().await?;
    redis::cmd("PING").query_async::<()>(&mut conn).await?;
    Ok(client)
}