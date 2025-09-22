use anyhow::Result;
use kube::Client;
use tracing::info;

mod types;
mod config;
mod parsing;
mod slack;
mod kubernetes;
mod metrics;
mod collector;
mod report;

use config::load_config;
use slack::{build_slack_payload, send_to_slack};
use kubernetes::ensure_metrics_available;
use collector::MetricsCollector;
use report::HealthReport;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cfg = load_config()?;
    info!("namespaces = {:?}", cfg.namespaces);

    let client = Client::try_default().await?;

    // Check metrics API availability early (fail fast if requested)
    if cfg.fail_if_no_metrics { 
        ensure_metrics_available(&client, &cfg.namespaces).await?; 
    }

    let collector = MetricsCollector::new(&client, &cfg);
    let mut report = HealthReport::new(cfg.clone());

    // Collect metrics for each namespace
    for ns in &cfg.namespaces {
        info!("Collecting metrics for namespace: {}", ns);
        
        // Collect pod metrics
        let pod_metrics = collector.collect_pod_metrics(ns).await?;
        report.add_pod_metrics(pod_metrics);

        // Collect job metrics
        let job_metrics = collector.collect_job_metrics(ns).await?;
        report.add_job_metrics(job_metrics);

        // Collect volume metrics
        let volume_metrics = collector.collect_volume_metrics(ns).await?;
        report.add_volume_metrics(volume_metrics);
    }

    // Collect cluster-wide metrics
    info!("Collecting cluster-wide metrics");
    let cluster_metrics = collector.collect_cluster_metrics().await?;
    report.set_cluster_metrics(cluster_metrics);

    // Log summary
    let summary = report.summary();
    info!("Health report summary: {} total issues found", summary.total_issues());

    // Send to Slack only if there are issues
    if summary.has_issues() {
        info!("Issues detected, sending notification to Slack");
        let payload = build_slack_payload(
            &report.config, 
            &report.pod_metrics.heavy_usage, 
            &report.pod_metrics.restarts, 
            &report.pod_metrics.pending,
            &report.pod_metrics.failed,
            &report.pod_metrics.unready,
            &report.pod_metrics.oom_killed,
            &report.cluster_metrics.problematic_nodes,
            &report.cluster_metrics.high_utilization_nodes,
            &report.volume_metrics.volume_issues,
            &report.job_metrics.failed_jobs,
            &report.job_metrics.missed_cronjobs
        );
        send_to_slack(&report.config.slack_webhook_url, &payload).await?;
    } else {
        info!("No issues detected, skipping Slack notification");
    }

    Ok(())
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .try_init();
}