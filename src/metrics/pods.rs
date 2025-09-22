use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use k8s_openapi::api::core::v1::{Container, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use kube::{api::ListParams, Api, Client};

use crate::types::{
    Config, PodRequestTotals, HeavyUsagePod, RestartEventInfo, PendingPodInfo,
    FailedPodInfo, UnreadyPodInfo, OomKilledInfo
};
use crate::parsing::{parse_cpu_to_millicores, parse_memory_to_bytes, compute_utilization_percentages, any_exceeds};
use super::base::{list_pod_metrics_http, build_usage_map_from_http, pod_status_time};

/// Analyze pods with heavy resource usage
pub async fn analyze_heavy_usage(
    client: &Client,
    namespace: &str,
    cfg: &Config,
) -> Result<Vec<HeavyUsagePod>> {
    let pods = list_namespace_pods(client, namespace).await?;
    analyze_heavy_usage_with_pods(client, namespace, cfg, &pods).await
}

/// Analyze pods with heavy resource usage using pre-listed pods
pub async fn analyze_heavy_usage_with_pods(
    client: &Client,
    namespace: &str,
    cfg: &Config,
    pods: &Vec<Pod>,
) -> Result<Vec<HeavyUsagePod>> {
    let metrics_items = list_pod_metrics_http(client, namespace).await?;
    let usage_by_pod = build_usage_map_from_http(metrics_items);
    
    let mut heavy_usage = Vec::new();
    
    for pod in pods.iter() {
        let pod_name = match pod.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };
        
        if let Some(usage) = usage_by_pod.get(&pod_name) {
            let requests = sum_requests(&pod);
            let (cpu_pct, mem_pct) = compute_utilization_percentages(usage, &requests);
            if let Some(exceeds) = any_exceeds(cpu_pct, mem_pct, cfg.threshold_percent) {
                if exceeds {
                    heavy_usage.push(HeavyUsagePod {
                        namespace: namespace.to_string(),
                        pod: pod_name,
                        cpu_pct,
                        mem_pct,
                    });
                }
            }
        }
    }
    
    Ok(heavy_usage)
}

/// Analyze container restarts beyond grace period
pub async fn analyze_restarts(
    client: &Client,
    namespace: &str,
    cfg: &Config,
) -> Result<Vec<RestartEventInfo>> {
    let pods = list_namespace_pods(client, namespace).await?;
    analyze_restarts_with_pods(namespace, cfg, &pods)
}

/// Analyze container restarts beyond grace period using pre-listed pods
pub fn analyze_restarts_with_pods(
    namespace: &str,
    cfg: &Config,
    pods: &Vec<Pod>,
) -> Result<Vec<RestartEventInfo>> {
    let mut restarts = Vec::new();
    
    for pod in pods.iter() {
        let pod_name = match pod.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };
        
        if let Some(statuses) = pod.status.as_ref().and_then(|s| s.container_statuses.as_ref()) {
            let startup_grace_cutoff = pod_status_time(&pod)
                .unwrap_or_else(Utc::now)
                + Duration::minutes(cfg.restart_grace_minutes);

            for cs in statuses {
                let restart_count = cs.restart_count;
                if restart_count > 0 {
                    let (last_restart_time, reason, message, exit_code) = extract_restart_info(cs);
                    let include = match last_restart_time {
                        Some(ts) => ts > startup_grace_cutoff,
                        None => {
                            // if no termination timestamp but container was waiting (e.g. CrashLoopBackOff), include if pod already past grace
                            Utc::now() > startup_grace_cutoff
                        }
                    };
                    if include {
                        restarts.push(RestartEventInfo {
                            namespace: namespace.to_string(),
                            pod: pod_name.clone(),
                            container: cs.name.clone(),
                            last_restart_time,
                            reason,
                            message,
                            exit_code,
                        });
                    }
                }
            }
        }
    }
    
    Ok(restarts)
}

/// Analyze pending pods beyond grace period
pub async fn analyze_pending_pods(
    client: &Client,
    namespace: &str,
    cfg: &Config,
) -> Result<Vec<PendingPodInfo>> {
    let pods = list_namespace_pods(client, namespace).await?;
    Ok(analyze_pending_pods_with_pods(namespace, cfg, &pods))
}

/// Analyze pending pods beyond grace period using pre-listed pods
pub fn analyze_pending_pods_with_pods(
    namespace: &str,
    cfg: &Config,
    pods: &Vec<Pod>,
) -> Vec<PendingPodInfo> {
    let mut pendings = Vec::new();
    
    for pod in pods.iter() {
        let pod_name = match pod.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };
        
        if is_pending_over_grace(&pod, cfg.pending_grace_minutes) {
            let since = pod_status_time(&pod).unwrap_or_else(Utc::now);
            let duration_minutes = (Utc::now() - since).num_minutes();
            pendings.push(PendingPodInfo {
                namespace: namespace.to_string(),
                pod: pod_name,
                since,
                duration_minutes,
            });
        }
    }
    pendings
}

/// Analyze failed pods with grace period consideration
pub async fn analyze_failed_pods(
    client: &Client,
    namespace: &str,
    cfg: &Config,
) -> Result<Vec<FailedPodInfo>> {
    let pods = list_namespace_pods(client, namespace).await?;
    Ok(analyze_failed_pods_with_pods(namespace, cfg, &pods))
}

/// Analyze failed pods using pre-listed pods
pub fn analyze_failed_pods_with_pods(
    namespace: &str,
    cfg: &Config,
    pods: &Vec<Pod>,
) -> Vec<FailedPodInfo> {
    let mut failed_pods = Vec::new();

    for pod in pods.iter() {
        let pod_name = match pod.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };

        if is_failed_over_grace(&pod, cfg.pending_grace_minutes) {
            let since = pod_status_time(&pod).unwrap_or_else(Utc::now);
            let duration_minutes = (Utc::now() - since).num_minutes();
            let (reason, message) = extract_pod_failure_info(&pod);

            failed_pods.push(FailedPodInfo {
                namespace: namespace.to_string(),
                pod: pod_name,
                since,
                duration_minutes,
                reason,
                message,
            });
        }
    }
    failed_pods
}

/// Analyze unready pods (readiness/liveness probe failures)
pub async fn analyze_unready_pods(
    client: &Client,
    namespace: &str,
    cfg: &Config,
) -> Result<Vec<UnreadyPodInfo>> {
    let pods = list_namespace_pods(client, namespace).await?;
    Ok(analyze_unready_pods_with_pods(namespace, cfg, &pods))
}

/// Analyze unready pods using pre-listed pods
pub fn analyze_unready_pods_with_pods(
    namespace: &str,
    cfg: &Config,
    pods: &Vec<Pod>,
) -> Vec<UnreadyPodInfo> {
    let mut unready_pods = Vec::new();

    for pod in pods.iter() {
        let pod_name = match pod.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };

        if is_unready_over_grace(&pod, cfg.pending_grace_minutes) {
            let since = pod_status_time(&pod).unwrap_or_else(Utc::now);
            let duration_minutes = (Utc::now() - since).num_minutes();
            let failed_conditions = extract_failed_conditions(&pod);

            unready_pods.push(UnreadyPodInfo {
                namespace: namespace.to_string(),
                pod: pod_name,
                since,
                duration_minutes,
                failed_conditions,
            });
        }
    }
    unready_pods
}

/// Analyze OOMKilled containers
pub async fn analyze_oom_killed(
    client: &Client,
    namespace: &str,
    cfg: &Config,
) -> Result<Vec<OomKilledInfo>> {
    let pods = list_namespace_pods(client, namespace).await?;
    Ok(analyze_oom_killed_with_pods(namespace, cfg, &pods))
}

/// Analyze OOMKilled containers using pre-listed pods
pub fn analyze_oom_killed_with_pods(
    namespace: &str,
    cfg: &Config,
    pods: &Vec<Pod>,
) -> Vec<OomKilledInfo> {
    let mut oom_killed = Vec::new();

    for pod in pods.iter() {
        let pod_name = match pod.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };

        if let Some(statuses) = pod.status.as_ref().and_then(|s| s.container_statuses.as_ref()) {
            let startup_grace_cutoff = pod_status_time(&pod)
                .unwrap_or_else(Utc::now)
                + Duration::minutes(cfg.restart_grace_minutes);

            for cs in statuses {
                if let Some(oom_info) = extract_oom_info(cs, &startup_grace_cutoff) {
                    oom_killed.push(OomKilledInfo {
                        namespace: namespace.to_string(),
                        pod: pod_name.clone(),
                        container: cs.name.clone(),
                        last_oom_time: oom_info.0,
                        restart_count: cs.restart_count,
                    });
                }
            }
        }
    }
    oom_killed
}

// Shared helper to list pods once per namespace
async fn list_namespace_pods(client: &Client, namespace: &str) -> Result<Vec<Pod>> {
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pods = pod_api.list(&ListParams::default()).await?;
    Ok(pods.items)
}

// Helper functions
fn is_pending_over_grace(pod: &Pod, grace_minutes: i64) -> bool {
    let phase = pod
        .status
        .as_ref()
        .and_then(|s| s.phase.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("");
    if phase != "Pending" {
        return false;
    }
    let since = pod_status_time(pod).unwrap_or_else(Utc::now);
    (Utc::now() - since) > Duration::minutes(grace_minutes)
}

fn is_failed_over_grace(pod: &Pod, grace_minutes: i64) -> bool {
    let phase = pod
        .status
        .as_ref()
        .and_then(|s| s.phase.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("");
    
    if phase != "Failed" {
        return false;
    }
    
    let since = pod_status_time(pod).unwrap_or_else(Utc::now);
    (Utc::now() - since) > Duration::minutes(grace_minutes)
}

fn is_unready_over_grace(pod: &Pod, grace_minutes: i64) -> bool {
    let phase = pod
        .status
        .as_ref()
        .and_then(|s| s.phase.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("");
    
    // Only check Running pods for readiness issues
    if phase != "Running" {
        return false;
    }
    
    let is_ready = pod
        .status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|conditions| {
            conditions.iter().any(|c| {
                c.type_ == "Ready" && c.status == "True"
            })
        })
        .unwrap_or(false);
    
    if is_ready {
        return false;
    }
    
    let since = pod_status_time(pod).unwrap_or_else(Utc::now);
    (Utc::now() - since) > Duration::minutes(grace_minutes)
}

fn sum_requests(pod: &Pod) -> PodRequestTotals {
    let mut cpu_sum: i64 = 0;
    let mut mem_sum: i64 = 0;
    let mut have_cpu = false;
    let mut have_mem = false;

    if let Some(spec) = pod.spec.as_ref() {
        let containers: &Vec<Container> = &spec.containers;
        for c in containers {
            if let Some(resources) = c.resources.as_ref() {
                if let Some(req) = resources.requests.as_ref() {
                    if let Some(cpu) = req.get("cpu").map(|q| q.0.as_str()) {
                        if let Some(mc) = parse_cpu_to_millicores(cpu) {
                            have_cpu = true;
                            cpu_sum += mc;
                        }
                    }
                    if let Some(mem) = req.get("memory").map(|q| q.0.as_str()) {
                        if let Some(bytes) = parse_memory_to_bytes(mem) {
                            have_mem = true;
                            mem_sum += bytes;
                        }
                    }
                }
            }
        }
    }

    PodRequestTotals {
        cpu_millicores: if have_cpu { Some(cpu_sum) } else { None },
        memory_bytes: if have_mem { Some(mem_sum) } else { None },
    }
}

fn extract_restart_info(cs: &k8s_openapi::api::core::v1::ContainerStatus) -> (Option<DateTime<Utc>>, Option<String>, Option<String>, Option<i32>) {
    // Prefer lastState.terminated
    if let Some(last_state) = cs.last_state.as_ref() {
        if let Some(term) = last_state.terminated.as_ref() {
            let ts = term.finished_at.as_ref().map(|t: &Time| t.0);
            let reason = term.reason.clone();
            let message = term.message.clone();
            let exit_code = term.exit_code;
            return (ts, reason, message, Some(exit_code));
        }
    }
    // Fallback to current waiting state (e.g., CrashLoopBackOff)
    if let Some(state) = cs.state.as_ref() {
        if let Some(wait) = state.waiting.as_ref() {
            return (None, wait.reason.clone(), wait.message.clone(), None);
        }
    }
    (None, None, None, None)
}

fn extract_pod_failure_info(pod: &Pod) -> (Option<String>, Option<String>) {
    let reason = pod
        .status
        .as_ref()
        .and_then(|s| s.reason.clone());
    
    let message = pod
        .status
        .as_ref()
        .and_then(|s| s.message.clone());
    
    (reason, message)
}

fn extract_failed_conditions(pod: &Pod) -> Vec<String> {
    pod.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|conditions| {
            conditions
                .iter()
                .filter(|c| c.status == "False")
                .map(|c| format!("{}: {}", c.type_, c.message.as_ref().unwrap_or(&"Unknown".to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn extract_oom_info(
    cs: &k8s_openapi::api::core::v1::ContainerStatus,
    grace_cutoff: &DateTime<Utc>,
) -> Option<(Option<DateTime<Utc>>,)> {
    // Check lastState.terminated for OOMKilled
    if let Some(last_state) = cs.last_state.as_ref() {
        if let Some(term) = last_state.terminated.as_ref() {
            if term.reason.as_ref().map(|r| r.as_str()) == Some("OOMKilled") {
                let ts = term.finished_at.as_ref().map(|t| t.0);
                if let Some(finish_time) = ts {
                    if finish_time > *grace_cutoff {
                        return Some((Some(finish_time),));
                    }
                } else if Utc::now() > *grace_cutoff {
                    return Some((None,));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use k8s_openapi::api::core::v1::{PodStatus, PodCondition, ContainerStatus, ContainerState, ContainerStateTerminated};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};

    fn create_test_config() -> Config {
        Config {
            namespaces: vec!["default".to_string()],
            threshold_percent: 85.0,
            slack_webhook_url: "https://test.com".to_string(),
            restart_grace_minutes: 5,
            pending_grace_minutes: 5,
            cluster_name: None,
            datacenter_name: None,
            fail_if_no_metrics: false,
        }
    }

    fn create_test_pod(name: &str, phase: &str, creation_time: DateTime<Utc>) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                creation_timestamp: Some(Time(creation_time)),
                ..Default::default()
            },
            status: Some(PodStatus {
                phase: Some(phase.to_string()),
                start_time: Some(Time(creation_time)),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn test_is_failed_over_grace() {
        let config = create_test_config();
        let old_time = Utc::now() - Duration::minutes(10);
        let recent_time = Utc::now() - Duration::minutes(2);

        // Test failed pod over grace period
        let mut failed_pod = create_test_pod("failed-pod", "Failed", old_time);
        assert!(is_failed_over_grace(&failed_pod, config.pending_grace_minutes));

        // Test failed pod within grace period
        failed_pod.metadata.creation_timestamp = Some(Time(recent_time));
        failed_pod.status.as_mut().unwrap().start_time = Some(Time(recent_time));
        assert!(!is_failed_over_grace(&failed_pod, config.pending_grace_minutes));

        // Test non-failed pod
        let running_pod = create_test_pod("running-pod", "Running", old_time);
        assert!(!is_failed_over_grace(&running_pod, config.pending_grace_minutes));
    }

    #[test]
    fn test_is_unready_over_grace() {
        let config = create_test_config();
        let old_time = Utc::now() - Duration::minutes(10);

        // Test unready running pod over grace period
        let mut unready_pod = create_test_pod("unready-pod", "Running", old_time);
        unready_pod.status.as_mut().unwrap().conditions = Some(vec![
            PodCondition {
                type_: "Ready".to_string(),
                status: "False".to_string(),
                message: Some("Container not ready".to_string()),
                ..Default::default()
            }
        ]);
        assert!(is_unready_over_grace(&unready_pod, config.pending_grace_minutes));

        // Test ready pod
        unready_pod.status.as_mut().unwrap().conditions = Some(vec![
            PodCondition {
                type_: "Ready".to_string(),
                status: "True".to_string(),
                ..Default::default()
            }
        ]);
        assert!(!is_unready_over_grace(&unready_pod, config.pending_grace_minutes));

        // Test non-running pod
        let pending_pod = create_test_pod("pending-pod", "Pending", old_time);
        assert!(!is_unready_over_grace(&pending_pod, config.pending_grace_minutes));
    }

    #[test]
    fn test_extract_pod_failure_info() {
        let mut pod = create_test_pod("test-pod", "Failed", Utc::now());
        
        // Test with reason and message
        pod.status.as_mut().unwrap().reason = Some("DeadlineExceeded".to_string());
        pod.status.as_mut().unwrap().message = Some("Job deadline exceeded".to_string());
        
        let (reason, message) = extract_pod_failure_info(&pod);
        assert_eq!(reason, Some("DeadlineExceeded".to_string()));
        assert_eq!(message, Some("Job deadline exceeded".to_string()));

        // Test without reason and message
        pod.status.as_mut().unwrap().reason = None;
        pod.status.as_mut().unwrap().message = None;
        
        let (reason, message) = extract_pod_failure_info(&pod);
        assert_eq!(reason, None);
        assert_eq!(message, None);
    }

    #[test]
    fn test_extract_failed_conditions() {
        let mut pod = create_test_pod("test-pod", "Running", Utc::now());
        
        // Test with failed conditions
        pod.status.as_mut().unwrap().conditions = Some(vec![
            PodCondition {
                type_: "Ready".to_string(),
                status: "False".to_string(),
                message: Some("Container not ready".to_string()),
                ..Default::default()
            },
            PodCondition {
                type_: "ContainersReady".to_string(),
                status: "False".to_string(),
                message: Some("Readiness probe failed".to_string()),
                ..Default::default()
            },
            PodCondition {
                type_: "PodScheduled".to_string(),
                status: "True".to_string(),
                ..Default::default()
            }
        ]);
        
        let failed_conditions = extract_failed_conditions(&pod);
        assert_eq!(failed_conditions.len(), 2);
        assert!(failed_conditions.contains(&"Ready: Container not ready".to_string()));
        assert!(failed_conditions.contains(&"ContainersReady: Readiness probe failed".to_string()));
        assert!(!failed_conditions.iter().any(|c| c.contains("PodScheduled")));
    }

    #[test]
    fn test_extract_oom_info() {
        let grace_cutoff = Utc::now() - Duration::minutes(2);
        let oom_time = Utc::now() - Duration::minutes(1); // After grace cutoff

        // Test OOMKilled container
        let mut container_status = ContainerStatus {
            name: "test-container".to_string(),
            restart_count: 5,
            last_state: Some(ContainerState {
                terminated: Some(ContainerStateTerminated {
                    reason: Some("OOMKilled".to_string()),
                    finished_at: Some(Time(oom_time)),
                    exit_code: 137,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let oom_info = extract_oom_info(&container_status, &grace_cutoff);
        assert!(oom_info.is_some());
        assert_eq!(oom_info.unwrap().0, Some(oom_time));

        // Test OOMKilled before grace period
        let early_oom_time = Utc::now() - Duration::minutes(10);
        container_status.last_state.as_mut().unwrap().terminated.as_mut().unwrap().finished_at = Some(Time(early_oom_time));
        
        let oom_info = extract_oom_info(&container_status, &grace_cutoff);
        assert!(oom_info.is_none());

        // Test non-OOMKilled container
        container_status.last_state.as_mut().unwrap().terminated.as_mut().unwrap().reason = Some("Error".to_string());
        
        let oom_info = extract_oom_info(&container_status, &grace_cutoff);
        assert!(oom_info.is_none());
    }
}
