use anyhow::Result;
use k8s_openapi::api::core::v1::Pod;
use kube::{api::ListParams, Api, Client};

use crate::types::{VolumeIssueInfo, VolumeIssueType};

/// Analyze volume issues (high usage and mount failures)
pub async fn analyze_volume_issues(
    client: &Client,
    namespace: &str,
    _volume_threshold_percent: f64,
) -> Result<Vec<VolumeIssueInfo>> {
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pods = pod_api.list(&ListParams::default()).await?;
    let mut volume_issues = Vec::new();

    for pod in pods.items {
        let pod_name = match pod.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };

        // Check for mount failures in events or container statuses
        if let Some(mount_failures) = extract_mount_failures(&pod) {
            for (volume_name, message) in mount_failures {
                volume_issues.push(VolumeIssueInfo {
                    namespace: namespace.to_string(),
                    pod: pod_name.clone(),
                    volume_name,
                    issue_type: VolumeIssueType::MountFailure,
                    message,
                });
            }
        }

        // TODO: Add volume usage monitoring when metrics are available
        // This would require additional metrics from kubelet or volume plugins
    }

    Ok(volume_issues)
}

fn extract_mount_failures(pod: &Pod) -> Option<Vec<(String, String)>> {
    let mut mount_failures = Vec::new();
    
    // Check container statuses for mount-related waiting reasons
    if let Some(statuses) = pod.status.as_ref().and_then(|s| s.container_statuses.as_ref()) {
        for status in statuses {
            if let Some(state) = status.state.as_ref() {
                if let Some(waiting) = state.waiting.as_ref() {
                    if let Some(reason) = waiting.reason.as_ref() {
                        if reason.contains("Mount") || reason.contains("Volume") {
                            let message = waiting.message.as_ref()
                                .cloned()
                                .unwrap_or_else(|| reason.clone());
                            mount_failures.push((format!("container-{}", status.name), message));
                        }
                    }
                }
            }
        }
    }

    if mount_failures.is_empty() {
        None
    } else {
        Some(mount_failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{ContainerStatus, ContainerState, ContainerStateWaiting, PodStatus};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use chrono::Utc;

    fn create_test_pod(name: &str) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn test_extract_mount_failures() {
        let mut pod = create_test_pod("test-pod");
        
        // Test with mount failure
        pod.status = Some(PodStatus {
            container_statuses: Some(vec![
                ContainerStatus {
                    name: "test-container".to_string(),
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("VolumeMount".to_string()),
                            message: Some("Failed to mount volume".to_string()),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            ]),
            ..Default::default()
        });

        let mount_failures = extract_mount_failures(&pod);
        assert!(mount_failures.is_some());
        let failures = mount_failures.unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "container-test-container");
        assert_eq!(failures[0].1, "Failed to mount volume");

        // Test with no mount failures
        pod.status.as_mut().unwrap().container_statuses = Some(vec![
            ContainerStatus {
                name: "test-container".to_string(),
                state: Some(ContainerState {
                    waiting: Some(ContainerStateWaiting {
                        reason: Some("ImagePullBackOff".to_string()),
                        message: Some("Failed to pull image".to_string()),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }
        ]);

        let mount_failures = extract_mount_failures(&pod);
        assert!(mount_failures.is_none());
    }

    #[test]
    fn test_extract_mount_failures_multiple_containers() {
        let mut pod = create_test_pod("test-pod");
        
        // Test with multiple containers, some with mount failures
        pod.status = Some(PodStatus {
            container_statuses: Some(vec![
                ContainerStatus {
                    name: "container1".to_string(),
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("VolumeMount".to_string()),
                            message: Some("Mount failed for volume1".to_string()),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ContainerStatus {
                    name: "container2".to_string(),
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("CreateContainerConfigError".to_string()),
                            message: Some("Config error".to_string()),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ContainerStatus {
                    name: "container3".to_string(),
                    state: Some(ContainerState {
                        waiting: Some(ContainerStateWaiting {
                            reason: Some("FailedMount".to_string()),
                            message: Some("Unable to attach volume".to_string()),
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            ]),
            ..Default::default()
        });

        let mount_failures = extract_mount_failures(&pod);
        assert!(mount_failures.is_some());
        let failures = mount_failures.unwrap();
        assert_eq!(failures.len(), 2);
        
        // Check that we got the mount-related failures
        assert!(failures.iter().any(|(name, _)| name == "container-container1"));
        assert!(failures.iter().any(|(name, _)| name == "container-container3"));
        assert!(!failures.iter().any(|(name, _)| name == "container-container2"));
    }
}
