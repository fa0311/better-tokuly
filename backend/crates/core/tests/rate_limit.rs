//! The rate limiter against a real Valkey (`TOKULY_VALKEY_URL`, as exported
//! by `tools/dev/infra.sh env`).
use std::time::Duration;

use redis::AsyncCommands as _;
use tokuly_core::cache::Cache;
use uuid::Uuid;

const WINDOW: Duration = Duration::from_secs(60);

async fn cache() -> Cache {
    let url = std::env::var("TOKULY_VALKEY_URL").unwrap_or_else(|_| "redis://localhost:6379".to_owned());
    Cache::connect(&url).await.expect("Valkey is reachable")
}

#[tokio::test]
async fn a_window_allows_its_hits_and_refuses_the_next() {
    let cache = cache().await;
    let key = format!("test:{}", Uuid::now_v7());
    for _ in 0..3 {
        cache.rate_limit(&key, 3, WINDOW).await.expect("within the limit");
    }
    assert_eq!(cache.rate_limit(&key, 3, WINDOW).await.unwrap_err().reason, "RATE_LIMITED");
    let ttl: i64 = cache.conn().ttl(format!("rl:{key}")).await.unwrap();
    assert!((1..=60).contains(&ttl), "the window ends: {ttl}");
    cache.del(&format!("rl:{key}")).await.unwrap();
}

/// What a process stopped between `INCR` and `EXPIRE` used to leave behind:
/// a counter over its limit that never expires.
#[tokio::test]
async fn a_counter_left_without_an_expiry_gets_one() {
    let cache = cache().await;
    let key = format!("test:{}", Uuid::now_v7());
    cache.conn().set::<_, _, ()>(format!("rl:{key}"), 50).await.unwrap();

    assert_eq!(cache.rate_limit(&key, 3, WINDOW).await.unwrap_err().reason, "RATE_LIMITED");
    let ttl: i64 = cache.conn().ttl(format!("rl:{key}")).await.unwrap();
    assert!((1..=60).contains(&ttl), "the lock-out ends with the window: {ttl}");
    cache.del(&format!("rl:{key}")).await.unwrap();
}

#[tokio::test]
async fn concurrent_hits_are_all_counted() {
    let cache = cache().await;
    let key = format!("test:{}", Uuid::now_v7());
    let hits = futures::future::join_all((0..20).map(|_| cache.rate_limit(&key, 12, WINDOW))).await;
    assert_eq!(hits.iter().filter(|hit| hit.is_ok()).count(), 12);
    cache.del(&format!("rl:{key}")).await.unwrap();
}

#[tokio::test]
async fn switched_off_limits_count_nothing() {
    let cache = cache().await.with_rate_limits(false);
    let key = format!("test:{}", Uuid::now_v7());
    for _ in 0..5 {
        cache.rate_limit(&key, 1, WINDOW).await.expect("limits are off");
    }
    assert!(!cache.exists(&format!("rl:{key}")).await.unwrap());
}
