use anyhow::Result;
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Node;
use kube::{api::ListParams, Api, Client};
use k8s_openapi::api::core::v1::Pod;

use crate::types::{ProblematicNodeInfo, NodeUtilizationInfo};
use crate::parsing::{parse_cpu_to_millicores, parse_memory_to_bytes};

/// Analyze problematic nodes
pub async fn analyze_problematic_nodes(client: &Client) -> Result<Vec<ProblematicNodeInfo>> {
    let node_api: Api<Node> = Api::all(client.clone());
    let nodes = node_api.list(&ListParams::default()).await?;
    let mut problematic_nodes = Vec::new();

    for node in nodes.items {
        let node_name = match node.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };

        let problematic_conditions = extract_problematic_conditions(&node);
        if !problematic_conditions.is_empty() {
            let since = node_condition_since(&node).unwrap_or_else(Utc::now);
            problematic_nodes.push(ProblematicNodeInfo {
                name: node_name,
                conditions: problematic_conditions,
                since,
            });
        }
    }

    Ok(problematic_nodes)
}

/// Analyze node utilization
pub async fn analyze_node_utilization(
    client: &Client,
    threshold_percent: f64,
    target_namespaces: &[String],
) -> Result<Vec<NodeUtilizationInfo>> {
    let node_api: Api<Node> = Api::all(client.clone());
    let nodes = node_api.list(&ListParams::default()).await?;
    let mut high_utilization_nodes = Vec::new();

    // Get node metrics
    let node_metrics = list_node_metrics_http(client).await?;
    let metrics_by_node = build_node_metrics_map(node_metrics);

    for node in nodes.items {
        let node_name = match node.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };

        let (pods_count, pods_capacity) = (
            count_scheduled_pods_on_node(client, &node_name, target_namespaces).await.unwrap_or(0),
            extract_node_pod_capacity(&node)
        );
        let (cpu_pct, memory_pct) = if let Some(metrics) = metrics_by_node.get(&node_name) {
            calculate_node_utilization_percentages(&node, metrics)
        } else {
            (None, None)
        };

        // Check if node exceeds thresholds
        let exceeds_threshold = cpu_pct.map(|c| c > threshold_percent).unwrap_or(false) ||
                              memory_pct.map(|m| m > threshold_percent).unwrap_or(false) ||
                              pods_capacity > 0 && (pods_count as f64 / pods_capacity as f64 * 100.0) > threshold_percent;

        if exceeds_threshold {
            high_utilization_nodes.push(NodeUtilizationInfo {
                name: node_name,
                cpu_pct,
                memory_pct,
                pods_count,
                pods_capacity,
            });
        }
    }

    Ok(high_utilization_nodes)
}

// Node metrics structures
#[derive(Debug, serde::Deserialize)]
struct NodeMetricsItem {
    metadata: serde_json::Value,
    usage: std::collections::HashMap<String, String>,
}

#[derive(Debug, serde::Deserialize)]
struct NodeMetricsList {
    items: Vec<NodeMetricsItem>,
}

async fn list_node_metrics_http(client: &Client) -> Result<Vec<NodeMetricsItem>> {
    use http::Request as HttpRequest;
    let path = "/apis/metrics.k8s.io/v1beta1/nodes";
    let req = HttpRequest::builder()
        .method("GET")
        .uri(path)
        .body(Vec::new())
        .map_err(|e| anyhow::anyhow!("build request: {}", e))?;
    let list: NodeMetricsList = client.request(req).await?;
    Ok(list.items)
}

fn build_node_metrics_map(items: Vec<NodeMetricsItem>) -> std::collections::HashMap<String, NodeMetricsItem> {
    let mut map = std::collections::HashMap::new();
    for item in items {
        let name = item
            .metadata
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !name.is_empty() {
            map.insert(name, item);
        }
    }
    map
}

fn extract_problematic_conditions(node: &Node) -> Vec<String> {
    node.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|conditions| {
            let mut problematic = Vec::new();
            
            for condition in conditions {
                match condition.type_.as_str() {
                    "Ready" => {
                        if condition.status != "True" {
                            problematic.push("NotReady".to_string());
                        }
                    }
                    "MemoryPressure" | "DiskPressure" | "PIDPressure" => {
                        if condition.status == "True" {
                            problematic.push(condition.type_.clone());
                        }
                    }
                    _ => {}
                }
            }
            
            problematic
        })
        .unwrap_or_default()
}

fn node_condition_since(node: &Node) -> Option<DateTime<Utc>> {
    node.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|c| c.type_ == "Ready")
                .and_then(|c| c.last_transition_time.as_ref())
                .map(|t| t.0)
        })
}

fn extract_node_pod_capacity(node: &Node) -> i32 {
    node.status
        .as_ref()
        .and_then(|s| s.capacity.as_ref())
        .and_then(|c| c.get("pods"))
        .and_then(|p| p.0.parse::<i32>().ok())
        .unwrap_or(0)
}

async fn count_scheduled_pods_on_node(client: &Client, node_name: &str, target_namespaces: &[String]) -> Result<i32> {
    // Count pods scheduled on the node restricted to target namespaces
    let lp = ListParams::default().fields(&format!("spec.nodeName={}", node_name));
    let mut total = 0usize;
    for ns in target_namespaces {
        let pod_api: Api<Pod> = Api::namespaced(client.clone(), ns);
        let pods = pod_api.list(&lp).await?;
        total += pods.items.len();
    }
    Ok(total as i32)
}

fn calculate_node_utilization_percentages(
    node: &Node, 
    metrics: &NodeMetricsItem
) -> (Option<f64>, Option<f64>) {
    let cpu_pct = if let (Some(cpu_usage), Some(cpu_capacity)) = (
        metrics.usage.get("cpu").and_then(|c| parse_cpu_to_millicores(c)),
        node.status.as_ref()
            .and_then(|s| s.capacity.as_ref())
            .and_then(|c| c.get("cpu"))
            .and_then(|c| parse_cpu_to_millicores(&c.0))
    ) {
        if cpu_capacity > 0 {
            Some((cpu_usage as f64 / cpu_capacity as f64) * 100.0)
        } else {
            None
        }
    } else {
        None
    };

    let memory_pct = if let (Some(memory_usage), Some(memory_capacity)) = (
        metrics.usage.get("memory").and_then(|m| parse_memory_to_bytes(m)),
        node.status.as_ref()
            .and_then(|s| s.capacity.as_ref())
            .and_then(|c| c.get("memory"))
            .and_then(|m| parse_memory_to_bytes(&m.0))
    ) {
        if memory_capacity > 0 {
            Some((memory_usage as f64 / memory_capacity as f64) * 100.0)
        } else {
            None
        }
    } else {
        None
    };

    (cpu_pct, memory_pct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{NodeStatus, NodeCondition};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use std::collections::BTreeMap;

    #[test]
    fn test_extract_problematic_conditions() {
        let mut node = Node {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                ..Default::default()
            },
            status: Some(NodeStatus {
                conditions: Some(vec![
                    NodeCondition {
                        type_: "Ready".to_string(),
                        status: "False".to_string(), // NotReady
                        ..Default::default()
                    },
                    NodeCondition {
                        type_: "MemoryPressure".to_string(),
                        status: "True".to_string(), // Problematic
                        ..Default::default()
                    },
                    NodeCondition {
                        type_: "DiskPressure".to_string(),
                        status: "False".to_string(), // OK
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let problematic_conditions = extract_problematic_conditions(&node);
        assert_eq!(problematic_conditions.len(), 2);
        assert!(problematic_conditions.contains(&"NotReady".to_string()));
        assert!(problematic_conditions.contains(&"MemoryPressure".to_string()));
        assert!(!problematic_conditions.contains(&"DiskPressure".to_string()));

        // Test healthy node
        node.status.as_mut().unwrap().conditions = Some(vec![
            NodeCondition {
                type_: "Ready".to_string(),
                status: "True".to_string(),
                ..Default::default()
            },
            NodeCondition {
                type_: "MemoryPressure".to_string(),
                status: "False".to_string(),
                ..Default::default()
            },
        ]);

        let problematic_conditions = extract_problematic_conditions(&node);
        assert!(problematic_conditions.is_empty());
    }

    #[test]
    fn test_extract_node_pod_info() {
        let mut capacity = BTreeMap::new();
        capacity.insert("pods".to_string(), Quantity("110".to_string()));
        
        let mut allocatable = BTreeMap::new();
        allocatable.insert("pods".to_string(), Quantity("100".to_string()));

        let node = Node {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                ..Default::default()
            },
            status: Some(NodeStatus {
                capacity: Some(capacity),
                allocatable: Some(allocatable),
                ..Default::default()
            }),
            ..Default::default()
        };

        let pods_capacity = extract_node_pod_capacity(&node);
        assert_eq!(pods_capacity, 110);  // capacity
    }

    #[test] 
    fn test_calculate_node_utilization_percentages() {
        // Create node with capacity
        let mut capacity = BTreeMap::new();
        capacity.insert("cpu".to_string(), Quantity("4".to_string())); // 4 cores = 4000m
        capacity.insert("memory".to_string(), Quantity("8Gi".to_string()));
        
        let node = Node {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                ..Default::default()
            },
            status: Some(NodeStatus {
                capacity: Some(capacity),
                ..Default::default()
            }),
            ..Default::default()
        };

        // Create metrics showing 50% usage
        let mut usage = std::collections::HashMap::new();
        usage.insert("cpu".to_string(), "2000m".to_string()); // 2 cores
        usage.insert("memory".to_string(), "4Gi".to_string()); // 4GB
        
        let metrics = NodeMetricsItem {
            metadata: serde_json::json!({"name": "test-node"}),
            usage,
        };

        let (cpu_pct, memory_pct) = calculate_node_utilization_percentages(&node, &metrics);
        
        assert!(cpu_pct.is_some());
        assert!((cpu_pct.unwrap() - 50.0).abs() < 0.1);
        
        assert!(memory_pct.is_some());
        assert!((memory_pct.unwrap() - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_node_condition_since() {
        let transition_time = Utc::now() - chrono::Duration::minutes(30);
        
        let node = Node {
            metadata: ObjectMeta {
                name: Some("test-node".to_string()),
                ..Default::default()
            },
            status: Some(NodeStatus {
                conditions: Some(vec![
                    NodeCondition {
                        type_: "Ready".to_string(),
                        status: "True".to_string(),
                        last_transition_time: Some(Time(transition_time)),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let since = node_condition_since(&node);
        assert!(since.is_some());
        assert_eq!(since.unwrap(), transition_time);
    }
}
