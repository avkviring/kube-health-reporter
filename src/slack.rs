use anyhow::{anyhow, Context, Result};
use tracing::error;
use crate::types::{
    Config, SlackPayload, HeavyUsagePod, RestartEventInfo, PendingPodInfo,
    FailedPodInfo, UnreadyPodInfo, OomKilledInfo, ProblematicNodeInfo, 
    NodeUtilizationInfo, VolumeIssueInfo, VolumeIssueType, FailedJobInfo, MissedCronJobInfo
};

pub fn build_slack_payload(
    cfg: &Config,
    heavy: &[HeavyUsagePod],
    restarts: &[RestartEventInfo],
    pendings: &[PendingPodInfo],
    failed: &[FailedPodInfo],
    unready: &[UnreadyPodInfo],
    oom_killed: &[OomKilledInfo],
    problematic_nodes: &[ProblematicNodeInfo],
    high_util_nodes: &[NodeUtilizationInfo],
    volume_issues: &[VolumeIssueInfo],
    failed_jobs: &[FailedJobInfo],
    missed_cronjobs: &[MissedCronJobInfo],
) -> SlackPayload {
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let title = match (&cfg.cluster_name, &cfg.datacenter_name) {
        (Some(c), Some(d)) => format!("Kubernetes Health Report - {} ({})", c, d),
        (Some(c), None) => format!("Kubernetes Health Report - {}", c),
        (None, Some(d)) => format!("Kubernetes Health Report - {}", d),
        (None, None) => "Kubernetes Health Report".to_string(),
    };
    blocks.push(serde_json::json!({
        "type": "header",
        "text": {"type": "plain_text", "text": title}
    }));

    let ns_text = format!("Namespaces: {}\nThreshold: {}%\nGrace: restarts {}m, pending {}m",
        cfg.namespaces.join(", "),
        cfg.threshold_percent,
        cfg.restart_grace_minutes,
        cfg.pending_grace_minutes,
    );
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": ns_text}
    }));

    // Heavy usage section
    let mut heavy_lines: Vec<String> = Vec::new();
    for h in heavy {
        let cpu = h.cpu_pct.map(|v| format!("{:.0}%", v)).unwrap_or("-".to_string());
        let mem = h.mem_pct.map(|v| format!("{:.0}%", v)).unwrap_or("-".to_string());
        heavy_lines.push(format!("• `{}/{}:` CPU {} | MEM {}", h.namespace, h.pod, cpu, mem));
    }
    if heavy_lines.is_empty() {
        heavy_lines.push("No pods exceeding threshold.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*High resource usage*\n{}", heavy_lines.join("\n"))}
    }));

    // Restarts section
    let mut restart_lines: Vec<String> = Vec::new();
    for r in restarts {
        let t = r
            .last_restart_time
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_else(|| "-".to_string());
        let reason = r.reason.clone().unwrap_or_else(|| "unknown".to_string());
        let msg = r.message.clone().unwrap_or_default();
        let code = r
            .exit_code
            .map(|c| format!(" (exit {})", c))
            .unwrap_or_default();
        restart_lines.push(format!(
            "• `{}/{}` [{}] {}{} - {}",
            r.namespace, r.pod, r.container, reason, code, msg
        ));
        restart_lines.push(format!("  last: {}", t));
    }
    if restart_lines.is_empty() {
        restart_lines.push("No container restarts beyond grace.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*Container restarts*\n{}", restart_lines.join("\n"))}
    }));

    // Pending section
    let mut pending_lines: Vec<String> = Vec::new();
    for p in pendings {
        pending_lines.push(format!(
            "• `{}/{}` pending for {}m (since {})",
            p.namespace,
            p.pod,
            p.duration_minutes,
            p.since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ));
    }
    if pending_lines.is_empty() {
        pending_lines.push("No pending pods beyond grace.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*Pending pods*\n{}", pending_lines.join("\n"))}
    }));

    // Failed pods section
    let mut failed_lines: Vec<String> = Vec::new();
    for f in failed {
        let reason = f.reason.as_ref().map(|s| s.as_str()).unwrap_or("Unknown");
        let message = f.message.as_ref().map(|m| format!(" - {}", m)).unwrap_or_default();
        failed_lines.push(format!(
            "• `{}/{}` failed for {}m ({}{})",
            f.namespace,
            f.pod,
            f.duration_minutes,
            reason,
            message
        ));
    }
    if failed_lines.is_empty() {
        failed_lines.push("No failed pods beyond grace.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*Failed pods*\n{}", failed_lines.join("\n"))}
    }));

    // Unready pods section
    let mut unready_lines: Vec<String> = Vec::new();
    for u in unready {
        let conditions = if u.failed_conditions.is_empty() {
            "Unknown conditions".to_string()
        } else {
            u.failed_conditions.join(", ")
        };
        unready_lines.push(format!(
            "• `{}/{}` unready for {}m ({})",
            u.namespace,
            u.pod,
            u.duration_minutes,
            conditions
        ));
    }
    if unready_lines.is_empty() {
        unready_lines.push("No unready pods beyond grace.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*Unready pods*\n{}", unready_lines.join("\n"))}
    }));

    // OOMKilled containers section
    let mut oom_lines: Vec<String> = Vec::new();
    for o in oom_killed {
        let time_str = o.last_oom_time
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_else(|| "recent".to_string());
        oom_lines.push(format!(
            "• `{}/{}` [{}] OOMKilled (restarts: {}, last: {})",
            o.namespace,
            o.pod,
            o.container,
            o.restart_count,
            time_str
        ));
    }
    if oom_lines.is_empty() {
        oom_lines.push("No OOMKilled containers beyond grace.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*OOMKilled containers*\n{}", oom_lines.join("\n"))}
    }));

    // Problematic nodes section
    let mut node_problem_lines: Vec<String> = Vec::new();
    for n in problematic_nodes {
        node_problem_lines.push(format!(
            "• `{}` {} (since {})",
            n.name,
            n.conditions.join(", "),
            n.since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ));
    }
    if node_problem_lines.is_empty() {
        node_problem_lines.push("No problematic nodes.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*Problematic nodes*\n{}", node_problem_lines.join("\n"))}
    }));

    // High utilization nodes section
    let mut node_util_lines: Vec<String> = Vec::new();
    for n in high_util_nodes {
        let cpu = n.cpu_pct.map(|v| format!("{:.0}%", v)).unwrap_or("-".to_string());
        let mem = n.memory_pct.map(|v| format!("{:.0}%", v)).unwrap_or("-".to_string());
        let pod_util = if n.pods_capacity > 0 {
            format!("{:.0}%", (n.pods_count as f64 / n.pods_capacity as f64) * 100.0)
        } else {
            "-".to_string()
        };
        node_util_lines.push(format!(
            "• `{}` CPU {} | MEM {} | Pods {}/{} ({})",
            n.name, cpu, mem, n.pods_count, n.pods_capacity, pod_util
        ));
    }
    if node_util_lines.is_empty() {
        node_util_lines.push("No high utilization nodes.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*High utilization nodes*\n{}", node_util_lines.join("\n"))}
    }));

    // Volume issues section
    let mut volume_lines: Vec<String> = Vec::new();
    for v in volume_issues {
        let issue_desc = match &v.issue_type {
            VolumeIssueType::HighUsage(pct) => format!("High usage ({:.1}%)", pct),
            VolumeIssueType::MountFailure => "Mount failure".to_string(),
        };
        volume_lines.push(format!(
            "• `{}/{}` volume '{}': {} - {}",
            v.namespace,
            v.pod,
            v.volume_name,
            issue_desc,
            v.message
        ));
    }
    if volume_lines.is_empty() {
        volume_lines.push("No volume issues.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*Volume issues*\n{}", volume_lines.join("\n"))}
    }));

    // Failed jobs section
    let mut job_lines: Vec<String> = Vec::new();
    for j in failed_jobs {
        let time_str = j.last_failure_time
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_else(|| "unknown".to_string());
        let reason = j.reason.as_ref().map(|s| s.as_str()).unwrap_or("Unknown");
        job_lines.push(format!(
            "• `{}/{}` failed pods: {} (reason: {}, last failure: {})",
            j.namespace,
            j.job,
            j.failed_pods,
            reason,
            time_str
        ));
    }
    if job_lines.is_empty() {
        job_lines.push("No failed jobs.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*Failed jobs*\n{}", job_lines.join("\n"))}
    }));

    // Missed CronJobs section
    let mut cronjob_lines: Vec<String> = Vec::new();
    for c in missed_cronjobs {
        cronjob_lines.push(format!(
            "• `{}/{}` missed {} runs (last scheduled: {})",
            c.namespace,
            c.cronjob,
            c.missed_runs,
            c.last_schedule_time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        ));
    }
    if cronjob_lines.is_empty() {
        cronjob_lines.push("No missed CronJobs.".to_string());
    }
    blocks.push(serde_json::json!({
        "type": "section",
        "text": {"type": "mrkdwn", "text": format!("*Missed CronJobs*\n{}", cronjob_lines.join("\n"))}
    }));

    SlackPayload { text: None, blocks }
}

pub async fn send_to_slack(webhook_url: &str, payload: &SlackPayload) -> Result<()> {
    let client = reqwest::Client::new();
    let res = client
        .post(webhook_url)
        .json(payload)
        .send()
        .await
        .context("Failed to send Slack request")?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        error!("Slack webhook failed: {} - {}", status, body);
        return Err(anyhow!("Slack webhook returned non-success status"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_build_slack_payload_basic() {
        let config = Config {
            namespaces: vec!["default".to_string(), "kube-system".to_string()],
            threshold_percent: 85.0,
            slack_webhook_url: "https://hooks.slack.com/test".to_string(),
            restart_grace_minutes: 5,
            pending_grace_minutes: 5,
            cluster_name: Some("test-cluster".to_string()),
            datacenter_name: Some("us-east-1".to_string()),
            fail_if_no_metrics: true,
        };
        
        let heavy_usage = vec![
            HeavyUsagePod {
                namespace: "default".to_string(),
                pod: "heavy-pod".to_string(),
                cpu_pct: Some(90.0),
                mem_pct: Some(95.0),
            }
        ];
        
        let restarts = vec![
            RestartEventInfo {
                namespace: "default".to_string(),
                pod: "restart-pod".to_string(),
                container: "main".to_string(),
                last_restart_time: Some(Utc::now()),
                reason: Some("Error".to_string()),
                message: Some("Container crashed".to_string()),
                exit_code: Some(1),
            }
        ];
        
        let pendings = vec![
            PendingPodInfo {
                namespace: "default".to_string(),
                pod: "pending-pod".to_string(),
                since: Utc::now(),
                duration_minutes: 10,
            }
        ];
        
        let payload = build_slack_payload(&config, &heavy_usage, &restarts, &pendings, &[], &[], &[], &[], &[], &[], &[], &[]);
        
        // Check that payload has blocks
        assert!(!payload.blocks.is_empty());
        assert_eq!(payload.text, None);
        
        // Should have 13 blocks: header, config info, and 11 metric sections
        assert_eq!(payload.blocks.len(), 13);
        
        // Check header block contains cluster name and datacenter name
        let header = &payload.blocks[0];
        let header_text = header.get("text").unwrap().get("text").unwrap().as_str().unwrap();
        assert!(header_text.contains("test-cluster"));
        assert!(header_text.contains("us-east-1"));
    }

    #[test]
    fn test_build_slack_payload_empty() {
        let config = Config {
            namespaces: vec!["default".to_string()],
            threshold_percent: 85.0,
            slack_webhook_url: "https://hooks.slack.com/test".to_string(),
            restart_grace_minutes: 5,
            pending_grace_minutes: 5,
            cluster_name: None,
            datacenter_name: None,
            fail_if_no_metrics: true,
        };
        
        let payload = build_slack_payload(&config, &[], &[], &[], &[], &[], &[], &[], &[], &[], &[], &[]);
        
        // Should have 13 blocks: header, config info, and 11 metric sections
        assert_eq!(payload.blocks.len(), 13);
        
        // Check that empty sections show appropriate messages
        let heavy_section = &payload.blocks[2];
        let heavy_text = heavy_section.get("text").unwrap().get("text").unwrap().as_str().unwrap();
        assert!(heavy_text.contains("No pods exceeding threshold"));
        
        let restart_section = &payload.blocks[3];
        let restart_text = restart_section.get("text").unwrap().get("text").unwrap().as_str().unwrap();
        assert!(restart_text.contains("No container restarts beyond grace"));
        
        let pending_section = &payload.blocks[4];
        let pending_text = pending_section.get("text").unwrap().get("text").unwrap().as_str().unwrap();
        assert!(pending_text.contains("No pending pods beyond grace"));
    }
}
