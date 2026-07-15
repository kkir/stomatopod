//! Shared Stomatopod design system: pure presentational Dioxus components and
//! brand CSS tokens.
//!
//! Used by the dashboard (`stomatopod-web`) and the marketing site
//! (`stomatopod-www`). App-specific shells, charts, and API-bound widgets stay
//! in those crates.

pub mod button;
pub mod card;
pub mod skeleton;
pub mod stat;

pub use button::{Button, ButtonVariant};
pub use card::{Card, EmptyState, SectionHeader};
pub use skeleton::Skeleton;
pub use stat::{DeltaDir, DeltaInfo, StatTile};
