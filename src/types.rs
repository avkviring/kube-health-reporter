use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub namespaces: Vec<String>,
    pub threshold_percent: f64,
    pub slack_webhook_url: String,
    pub restart_grace_minutes: i64,
    pub pending_grace_minutes: i64,
    pub cluster_name: Option<String>,
    pub datacenter_name: Option<String>,
    pub fail_if_no_metrics: bool,
}

#[derive(Debug, Default, Clone)]
pub struct PodUsageTotals {
    pub cpu_millicores: i64,
    pub memory_bytes: i64,
}

#[derive(Debug, Default, Clone)]
pub struct PodRequestTotals {
    pub cpu_millicores: Option<i64>,
    pub memory_bytes: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct HeavyUsagePod {
    pub namespace: String,
    pub pod: String,
    pub cpu_pct: Option<f64>,
    pub mem_pct: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct RestartEventInfo {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub last_restart_time: Option<DateTime<Utc>>,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct PendingPodInfo {
    pub namespace: String,
    pub pod: String,
    pub since: DateTime<Utc>,
    pub duration_minutes: i64,
}

#[derive(Debug, Clone)]
pub struct FailedPodInfo {
    pub namespace: String,
    pub pod: String,
    pub since: DateTime<Utc>,
    pub duration_minutes: i64,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UnreadyPodInfo {
    pub namespace: String,
    pub pod: String,
    pub since: DateTime<Utc>,
    pub duration_minutes: i64,
    pub failed_conditions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OomKilledInfo {
    pub namespace: String,
    pub pod: String,
    pub container: String,
    pub last_oom_time: Option<DateTime<Utc>>,
    pub restart_count: i32,
}

#[derive(Debug, Clone)]
pub struct ProblematicNodeInfo {
    pub name: String,
    pub conditions: Vec<String>,
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NodeUtilizationInfo {
    pub name: String,
    pub cpu_pct: Option<f64>,
    pub memory_pct: Option<f64>,
    pub pods_count: i32,
    pub pods_capacity: i32,
}

#[derive(Debug, Clone)]
pub struct VolumeIssueInfo {
    pub namespace: String,
    pub pod: String,
    pub volume_name: String,
    pub issue_type: VolumeIssueType,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum VolumeIssueType {
    HighUsage(f64), // percentage
    MountFailure,
}

#[derive(Debug, Clone)]
pub struct FailedJobInfo {
    pub namespace: String,
    pub job: String,
    pub failed_pods: i32,
    pub last_failure_time: Option<DateTime<Utc>>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MissedCronJobInfo {
    pub namespace: String,
    pub cronjob: String,
    pub last_schedule_time: DateTime<Utc>,
    pub missed_runs: i32,
}

#[derive(Serialize)]
pub struct SlackPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub blocks: Vec<serde_json::Value>,
}
