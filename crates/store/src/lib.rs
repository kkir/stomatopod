//! Embedded analytics storage for the self-hosted appliance.
//!
//! SQLite metadata, write-ahead log, and Parquet event partitions under a local
//! data directory. Cloud / multi-tenant backends are not part of this crate;
//! implement `stomatopod_core::traits::{StorageBackend, MetaStore}` elsewhere.

pub mod embedded;

pub use embedded::EmbeddedBackend;
