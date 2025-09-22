use crate::types::*;
use crate::collector::{PodMetrics, JobMetrics, VolumeMetrics, ClusterMetrics};

/// Aggregated health report containing all metrics
pub struct HealthReport {
    pub config: Config,
    pub pod_metrics: AllNamespacePodMetrics,
    pub job_metrics: AllNamespaceJobMetrics,
    pub volume_metrics: AllNamespaceVolumeMetrics,
    pub cluster_metrics: ClusterMetrics,
}

/// Pod metrics aggregated across all namespaces
pub struct AllNamespacePodMetrics {
    pub heavy_usage: Vec<HeavyUsagePod>,
    pub restarts: Vec<RestartEventInfo>,
    pub pending: Vec<PendingPodInfo>,
    pub failed: Vec<FailedPodInfo>,
    pub unready: Vec<UnreadyPodInfo>,
    pub oom_killed: Vec<OomKilledInfo>,
}

/// Job metrics aggregated across all namespaces
pub struct AllNamespaceJobMetrics {
    pub failed_jobs: Vec<FailedJobInfo>,
    pub missed_cronjobs: Vec<MissedCronJobInfo>,
}

/// Volume metrics aggregated across all namespaces
pub struct AllNamespaceVolumeMetrics {
    pub volume_issues: Vec<VolumeIssueInfo>,
}

impl HealthReport {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            pod_metrics: AllNamespacePodMetrics {
                heavy_usage: Vec::new(),
                restarts: Vec::new(),
                pending: Vec::new(),
                failed: Vec::new(),
                unready: Vec::new(),
                oom_killed: Vec::new(),
            },
            job_metrics: AllNamespaceJobMetrics {
                failed_jobs: Vec::new(),
                missed_cronjobs: Vec::new(),
            },
            volume_metrics: AllNamespaceVolumeMetrics {
                volume_issues: Vec::new(),
            },
            cluster_metrics: ClusterMetrics {
                problematic_nodes: Vec::new(),
                high_utilization_nodes: Vec::new(),
            },
        }
    }

    pub fn add_pod_metrics(&mut self, metrics: PodMetrics) {
        self.pod_metrics.heavy_usage.extend(metrics.heavy_usage);
        self.pod_metrics.restarts.extend(metrics.restarts);
        self.pod_metrics.pending.extend(metrics.pending);
        self.pod_metrics.failed.extend(metrics.failed);
        self.pod_metrics.unready.extend(metrics.unready);
        self.pod_metrics.oom_killed.extend(metrics.oom_killed);
    }

    pub fn add_job_metrics(&mut self, metrics: JobMetrics) {
        self.job_metrics.failed_jobs.extend(metrics.failed_jobs);
        self.job_metrics.missed_cronjobs.extend(metrics.missed_cronjobs);
    }

    pub fn add_volume_metrics(&mut self, metrics: VolumeMetrics) {
        self.volume_metrics.volume_issues.extend(metrics.volume_issues);
    }

    pub fn set_cluster_metrics(&mut self, metrics: ClusterMetrics) {
        self.cluster_metrics = metrics;
    }

    /// Check if the report has any issues to report
    pub fn has_issues(&self) -> bool {
        !self.pod_metrics.heavy_usage.is_empty() ||
        !self.pod_metrics.restarts.is_empty() ||
        !self.pod_metrics.pending.is_empty() ||
        !self.pod_metrics.failed.is_empty() ||
        !self.pod_metrics.unready.is_empty() ||
        !self.pod_metrics.oom_killed.is_empty() ||
        !self.job_metrics.failed_jobs.is_empty() ||
        !self.job_metrics.missed_cronjobs.is_empty() ||
        !self.volume_metrics.volume_issues.is_empty() ||
        !self.cluster_metrics.problematic_nodes.is_empty() ||
        !self.cluster_metrics.high_utilization_nodes.is_empty()
    }

    /// Get a summary of the number of issues found
    pub fn summary(&self) -> ReportSummary {
        ReportSummary {
            heavy_usage_count: self.pod_metrics.heavy_usage.len(),
            restart_count: self.pod_metrics.restarts.len(),
            pending_count: self.pod_metrics.pending.len(),
            failed_pod_count: self.pod_metrics.failed.len(),
            unready_count: self.pod_metrics.unready.len(),
            oom_killed_count: self.pod_metrics.oom_killed.len(),
            failed_job_count: self.job_metrics.failed_jobs.len(),
            missed_cronjob_count: self.job_metrics.missed_cronjobs.len(),
            volume_issue_count: self.volume_metrics.volume_issues.len(),
            problematic_node_count: self.cluster_metrics.problematic_nodes.len(),
            high_util_node_count: self.cluster_metrics.high_utilization_nodes.len(),
        }
    }
}

pub struct ReportSummary {
    pub heavy_usage_count: usize,
    pub restart_count: usize,
    pub pending_count: usize,
    pub failed_pod_count: usize,
    pub unready_count: usize,
    pub oom_killed_count: usize,
    pub failed_job_count: usize,
    pub missed_cronjob_count: usize,
    pub volume_issue_count: usize,
    pub problematic_node_count: usize,
    pub high_util_node_count: usize,
}

impl ReportSummary {
    pub fn total_issues(&self) -> usize {
        self.heavy_usage_count +
        self.restart_count +
        self.pending_count +
        self.failed_pod_count +
        self.unready_count +
        self.oom_killed_count +
        self.failed_job_count +
        self.missed_cronjob_count +
        self.volume_issue_count +
        self.problematic_node_count +
        self.high_util_node_count
    }

    pub fn has_issues(&self) -> bool {
        self.total_issues() > 0
    }
}
