use anyhow::Result;
use kube::Client;

use crate::types::*;
use crate::metrics;

/// Collector structure that groups related metrics collection
pub struct MetricsCollector<'a> {
    client: &'a Client,
    config: &'a Config,
}

impl<'a> MetricsCollector<'a> {
    pub fn new(client: &'a Client, config: &'a Config) -> Self {
        Self { client, config }
    }

    /// Collect all pod-related metrics for a namespace
    pub async fn collect_pod_metrics(&self, namespace: &str) -> Result<PodMetrics> {
        // List pods once
        let pods = {
            use kube::{Api, api::ListParams};
            use k8s_openapi::api::core::v1::Pod;
            let pod_api: Api<Pod> = Api::namespaced(self.client.clone(), namespace);
            pod_api.list(&ListParams::default()).await?.items
        };

        // Run analyzers against the pre-listed pods
        let heavy_usage = metrics::pods::analyze_heavy_usage_with_pods(self.client, namespace, self.config, &pods).await?;
        let restarts = metrics::pods::analyze_restarts_with_pods(namespace, self.config, &pods)?;
        let pending = metrics::pods::analyze_pending_pods_with_pods(namespace, self.config, &pods);
        let failed = metrics::pods::analyze_failed_pods_with_pods(namespace, self.config, &pods);
        let unready = metrics::pods::analyze_unready_pods_with_pods(namespace, self.config, &pods);
        let oom_killed = metrics::pods::analyze_oom_killed_with_pods(namespace, self.config, &pods);

        Ok(PodMetrics {
            heavy_usage,
            restarts,
            pending,
            failed,
            unready,
            oom_killed,
        })
    }

    /// Collect all job-related metrics for a namespace
    pub async fn collect_job_metrics(&self, namespace: &str) -> Result<JobMetrics> {
        let failed_jobs = metrics::analyze_failed_jobs(self.client, namespace, self.config).await?;
        let missed_cronjobs = metrics::analyze_missed_cronjobs(
            self.client, 
            namespace, 
            self.config.pending_grace_minutes
        ).await?;

        Ok(JobMetrics {
            failed_jobs,
            missed_cronjobs,
        })
    }

    /// Collect all volume-related metrics for a namespace
    pub async fn collect_volume_metrics(&self, namespace: &str) -> Result<VolumeMetrics> {
        let volume_issues = metrics::analyze_volume_issues(
            self.client, 
            namespace, 
            85.0 // TODO: Make this configurable
        ).await?;

        Ok(VolumeMetrics {
            volume_issues,
        })
    }

    /// Collect all cluster-wide metrics
    pub async fn collect_cluster_metrics(&self) -> Result<ClusterMetrics> {
        let problematic_nodes = metrics::analyze_problematic_nodes(self.client).await?;
        let high_utilization_nodes = metrics::analyze_node_utilization(
            self.client, 
            self.config.threshold_percent,
            &self.config.namespaces,
        ).await?;

        Ok(ClusterMetrics {
            problematic_nodes,
            high_utilization_nodes,
        })
    }
}

/// Grouped pod metrics
pub struct PodMetrics {
    pub heavy_usage: Vec<HeavyUsagePod>,
    pub restarts: Vec<RestartEventInfo>,
    pub pending: Vec<PendingPodInfo>,
    pub failed: Vec<FailedPodInfo>,
    pub unready: Vec<UnreadyPodInfo>,
    pub oom_killed: Vec<OomKilledInfo>,
}

/// Grouped job metrics
pub struct JobMetrics {
    pub failed_jobs: Vec<FailedJobInfo>,
    pub missed_cronjobs: Vec<MissedCronJobInfo>,
}

/// Grouped volume metrics
pub struct VolumeMetrics {
    pub volume_issues: Vec<VolumeIssueInfo>,
}

/// Grouped cluster-wide metrics
pub struct ClusterMetrics {
    pub problematic_nodes: Vec<ProblematicNodeInfo>,
    pub high_utilization_nodes: Vec<NodeUtilizationInfo>,
}
