// Public modules
pub mod types;
pub mod config;
pub mod parsing;
pub mod slack;
pub mod kubernetes;
pub mod metrics;
pub mod collector;
pub mod report;

// Re-export commonly used items
pub use types::*;
pub use config::{load_config, load_config_with_env, EnvironmentProvider, SystemEnvironment, MockEnvironment};
pub use parsing::{parse_cpu_to_millicores, parse_memory_to_bytes, compute_utilization_percentages, any_exceeds};
pub use slack::{build_slack_payload, send_to_slack};
pub use kubernetes::{ensure_metrics_available, analyze_namespace};
pub use metrics::*;
pub use collector::MetricsCollector;
pub use report::{HealthReport, ReportSummary};
