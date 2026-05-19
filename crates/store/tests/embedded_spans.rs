use std::time::Duration;

use chrono::Utc;
use ulid::Ulid;

use stomatopod_core::{
    config::EmbeddedConfig,
    domain::agent_span::{AgentSpan, SpanKind},
    query::spans::SpanQuery,
    traits::AgentStore,
};
use stomatopod_store::embedded::EmbeddedBackend;

fn cfg(dir: &tempfile::TempDir) -> EmbeddedConfig {
    EmbeddedConfig {
        data_dir: dir.path().to_path_buf(),
        wal_fsync_interval_ms: 200,
        // Tiny thresholds so the flush worker writes parquet immediately.
        parquet_flush_rows: 1,
        parquet_flush_interval_s: 1,
    }
}

fn make_span(site_id: Ulid, agent_id: &str, session: &str, cost: f64) -> AgentSpan {
    let now = Utc::now();
    AgentSpan {
        id: Ulid::new(),
        site_id,
        agent_id: agent_id.into(),
        agent_session_id: session.into(),
        parent_span_id: None,
        kind: SpanKind::Request,
        model: "claude-opus-4-7".into(),
        started_at: now,
        ended_at: now + chrono::Duration::milliseconds(123),
        input_tokens: 100,
        output_tokens: 200,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        cost_usd: cost,
        tool_name: None,
        tool_input_hash: None,
        stop_reason: Some("end_turn".into()),
        properties: None,
    }
}

#[tokio::test]
async fn span_round_trip_and_session_cost() {
    let dir = tempfile::tempdir().unwrap();
    let backend = EmbeddedBackend::open(&cfg(&dir)).await.unwrap();
    let site_id = Ulid::new();

    let spans = vec![
        make_span(site_id, "agent-a", "sess-1", 0.12),
        make_span(site_id, "agent-a", "sess-1", 0.34),
        make_span(site_id, "agent-b", "sess-2", 1.00),
    ];
    backend.ingest_spans(spans).await.unwrap();

    // Wait for the flush worker (flush_interval_s=1 + grace).
    tokio::time::sleep(Duration::from_secs(2)).await;

    let now = Utc::now();
    let rows = backend
        .query_spans(&SpanQuery {
            site_id,
            agent_id: Some("agent-a".into()),
            session_id: None,
            since: now - chrono::Duration::minutes(5),
            until: now + chrono::Duration::minutes(5),
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.agent_id == "agent-a"));

    let session_cost = backend.session_cost_usd(site_id, "sess-1").await.unwrap();
    assert!((session_cost - 0.46).abs() < 1e-9, "got {session_cost}");

    let summaries = backend
        .summarize_agents(site_id, now - chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(summaries.len(), 2);
    let a = summaries.iter().find(|s| s.agent_id == "agent-a").unwrap();
    assert_eq!(a.total_spans, 2);
    assert_eq!(a.total_input_tokens, 200);
    assert_eq!(a.total_output_tokens, 400);
    assert!((a.total_cost_usd - 0.46).abs() < 1e-9);
}

#[tokio::test]
async fn span_wal_replay_recovers_unflushed_data() {
    let dir = tempfile::tempdir().unwrap();
    let site_id = Ulid::new();

    {
        let backend = EmbeddedBackend::open(&cfg(&dir)).await.unwrap();
        backend
            .ingest_spans(vec![make_span(site_id, "agent-c", "sess-x", 0.99)])
            .await
            .unwrap();
        // Let the worker drain the channel into the WAL.
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // Wipe parquet so the only durable record is the WAL. If the
    // implementation lost the WAL append we'd see 0.0 here.
    let parquet_root = dir.path().join("parquet_spans");
    if parquet_root.exists() {
        std::fs::remove_dir_all(&parquet_root).unwrap();
    }

    let backend = EmbeddedBackend::open(&cfg(&dir)).await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;

    let cost = backend.session_cost_usd(site_id, "sess-x").await.unwrap();
    assert!((cost - 0.99).abs() < 1e-9, "got {cost}");
}
