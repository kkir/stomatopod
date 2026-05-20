use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use std::sync::Arc;

/// Arrow schema for `AgentSpan`. Column order must match the
/// `RecordBatch` builder in `writer.rs`. Versioned via the
/// `parquet_spans/v1/` directory layout so new nullable columns can be
/// appended in a compatible way.
pub fn agent_span_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("site_id", DataType::Utf8, false),
        Field::new(
            "agent_id",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            false,
        ),
        Field::new("agent_session_id", DataType::Utf8, false),
        Field::new("parent_span_id", DataType::Utf8, true),
        Field::new(
            "kind",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            false,
        ),
        Field::new(
            "model",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            false,
        ),
        Field::new(
            "started_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new(
            "ended_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("input_tokens", DataType::UInt32, false),
        Field::new("output_tokens", DataType::UInt32, false),
        Field::new("cache_read_tokens", DataType::UInt32, false),
        Field::new("cache_creation_tokens", DataType::UInt32, false),
        Field::new("cost_usd", DataType::Float64, false),
        Field::new(
            "tool_name",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            true,
        ),
        Field::new("tool_input_hash", DataType::Utf8, true),
        Field::new(
            "stop_reason",
            DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
            true,
        ),
        Field::new("properties", DataType::Utf8, true),
    ]))
}
