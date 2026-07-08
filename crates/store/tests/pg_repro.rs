use chrono::Utc;
use std::time::Duration;
use stomatopod_core::domain::event::{DeviceType, Event, EventKind};
use stomatopod_core::query::pageviews::PageviewsQuery;
use stomatopod_core::traits::StorageBackend;
use stomatopod_store::postgres::PostgresBackend;
use ulid::Ulid;

#[tokio::test]
async fn test_pg_pageview() {
    // We would need a Postgres DB for this test. Maybe skip for now if it requires a running DB.
}
