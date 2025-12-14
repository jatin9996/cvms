use redis::{AsyncCommands, Client};
use std::sync::Arc;
use tracing::warn;

#[derive(Clone)]
pub struct Cache {
    client: Arc<Client>,
    ttl_seconds: u64,
}

impl Cache {
    pub fn new(redis_url: &str, ttl_seconds: u64) -> Result<Self, redis::RedisError> {
        let client = Client::open(redis_url)?;
        Ok(Self {
            client: Arc::new(client),
            ttl_seconds,
        })
    }

    pub async fn get_balance(&self, key: &str) -> Option<u64> {
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("Redis connection error: {}", e);
                return None;
            }
        };
        match conn.get::<_, String>(format!("balance:{}", key)).await {
            Ok(val) => val.parse().ok(),
            Err(e) => {
                if e.kind() == redis::ErrorKind::TypeError {
                    None
                } else {
                    warn!("Redis get error: {}", e);
                    None
                }
            }
        }
    }

    pub async fn set_balance(&self, key: &str, value: u64) {
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("Redis connection error: {}", e);
                return;
            }
        };
        if let Err(e) = conn
            .set_ex::<_, _, ()>(
                format!("balance:{}", key),
                value.to_string(),
                self.ttl_seconds,
            )
            .await
        {
            warn!("Redis set error: {}", e);
        }
    }

    pub async fn invalidate_balance(&self, key: &str) {
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("Redis connection error: {}", e);
                return;
            }
        };
        if let Err(e) = conn.del::<_, ()>(format!("balance:{}", key)).await {
            warn!("Redis delete error: {}", e);
        }
    }

    pub async fn get_tvl(&self) -> Option<i64> {
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("Redis connection error: {}", e);
                return None;
            }
        };
        match conn.get::<_, String>("tvl".to_string()).await {
            Ok(val) => val.parse().ok(),
            Err(e) => {
                if e.kind() == redis::ErrorKind::TypeError {
                    None
                } else {
                    warn!("Redis get error: {}", e);
                    None
                }
            }
        }
    }

    pub async fn set_tvl(&self, value: i64) {
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                warn!("Redis connection error: {}", e);
                return;
            }
        };
        if let Err(e) = conn
            .set_ex::<_, _, ()>("tvl".to_string(), value.to_string(), 60)
            .await
        {
            warn!("Redis set error: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires Redis server
    async fn test_cache_operations() {
        let cache = Cache::new("redis://localhost:6379", 60).unwrap();
        let key = "test_owner";
        
        // Test set and get
        cache.set_balance(key, 1000).await;
        let balance = cache.get_balance(key).await;
        assert_eq!(balance, Some(1000));
        
        // Test invalidation
        cache.invalidate_balance(key).await;
        let balance_after = cache.get_balance(key).await;
        assert_eq!(balance_after, None);
    }
}
