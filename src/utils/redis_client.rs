use crate::settings::CONFIG;
use redis::{Client, aio::MultiplexedConnection};

pub async fn redis_connection() -> MultiplexedConnection {
    let client = Client::open(CONFIG.redis_url.clone()).expect("Invalid Redis URL");

    client
        .get_multiplexed_async_connection()
        .await
        .expect("Failed to connect to Redis")
}
