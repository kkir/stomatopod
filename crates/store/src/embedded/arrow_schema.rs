use arrow::{
    datatypes::{DataType, Field, Schema, TimeUnit},
};
use std::sync::Arc;

/// Arrow schema for the `Event` struct.
/// Column order must match the RecordBatch builder in `writer.rs`.
pub fn event_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("site_id", DataType::Utf8, false),
        Field::new("name", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), false),
        Field::new("kind", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), false),
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new(
            "received_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("url", DataType::Utf8, false),
        Field::new("referrer", DataType::Utf8, true),
        Field::new("utm_source", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), true),
        Field::new("utm_medium", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), true),
        Field::new("utm_campaign", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), true),
        Field::new("utm_term", DataType::Utf8, true),
        Field::new("utm_content", DataType::Utf8, true),
        Field::new("browser", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), false),
        Field::new("browser_version", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), false),
        Field::new("os", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), false),
        Field::new("os_version", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), false),
        Field::new("device_type", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), false),
        Field::new("screen_width", DataType::UInt16, true),
        Field::new("screen_height", DataType::UInt16, true),
        Field::new("language", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), true),
        Field::new("ip_anonymized", DataType::Utf8, false),
        Field::new("country_code", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), true),
        Field::new("region", DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)), true),
        Field::new("city", DataType::Utf8, true),
        Field::new("session_id", DataType::FixedSizeBinary(16), false),
        Field::new("properties", DataType::Utf8, true),
    ]))
}
