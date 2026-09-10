//! Portable application and device metrics for the Pheme VA.
//!
//! Core code publishes typed per-run events into this crate. Hosts can attach
//! synchronous subscribers for a TUI, tests, local aggregation, or an API
//! exporter. The crate intentionally has no HTTP, Go, UI, or mobile lifecycle
//! dependency.

mod batch;
mod context;
mod energy;
mod event;
mod hub;
mod resources;

pub use batch::MetricsBatcher;
pub use context::{MetricsConfig, MetricsContext, MetricsTimer};
pub use energy::EnergyAccumulator;
pub use event::{
    MetricBatch, MetricEvent, MetricSample, MetricScope, MetricUnit, MetricValue, Stage,
    METRICS_SCHEMA_VERSION,
};
pub use hub::{MetricsHub, MetricsSubscriber, MetricsSubscription};
#[cfg(feature = "desktop")]
pub use resources::SysinfoResourceSampler;
pub use resources::{
    Measurement, ResourceCapabilities, ResourceCollector, ResourceSampler, ResourceSnapshot,
};
