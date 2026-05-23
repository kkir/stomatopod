// `SpanBuffer` is a `Buffer<AgentSpan>` alias defined in the parent module
// alongside `EventBuffer`. Re-exported here so the `super::buffer::SpanBuffer`
// path the rest of the spans pipeline uses keeps working.
pub use crate::embedded::buffer::SpanBuffer;
