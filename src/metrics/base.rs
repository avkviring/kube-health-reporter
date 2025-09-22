use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::Pod;
use kube::Client;
use serde::Deserialize;
use std::collections::HashMap;

use crate::types::PodUsageTotals;
use crate::parsing::{parse_cpu_to_millicores, parse_memory_to_bytes};

#[derive(Debug, Deserialize)]
pub struct ContainerMetrics { 
    pub name: String, 
    pub usage: HashMap<String, String> 
}

#[derive(Debug, Deserialize)]
pub struct PodMetricsItem { 
    pub metadata: serde_json::Value, 
    pub containers: Vec<ContainerMetrics> 
}

#[derive(Debug, Deserialize)]
pub struct PodMetricsList { 
    pub items: Vec<PodMetricsItem> 
}

pub async fn list_pod_metrics_http(client: &Client, namespace: &str) -> Result<Vec<PodMetricsItem>> {
    use http::Request as HttpRequest;
    let path = format!("/apis/metrics.k8s.io/v1beta1/namespaces/{}/pods", namespace);
    let req = HttpRequest::builder()
        .method("GET")
        .uri(path)
        .body(Vec::new())
        .map_err(|e| anyhow!("build request: {}", e))?;
    let list: PodMetricsList = client.request(req).await?;
    Ok(list.items)
}

pub fn build_usage_map_from_http(items: Vec<PodMetricsItem>) -> HashMap<String, PodUsageTotals> {
    let mut map = HashMap::new();
    for item in items {
        let name = item
            .metadata
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() { continue; }
        let mut totals = PodUsageTotals::default();
        for c in item.containers {
            if let Some(cpu_q) = c.usage.get("cpu") {
                if let Some(mc) = parse_cpu_to_millicores(cpu_q) { totals.cpu_millicores += mc; }
            }
            if let Some(mem_q) = c.usage.get("memory") {
                if let Some(bytes) = parse_memory_to_bytes(mem_q) { totals.memory_bytes += bytes; }
            }
        }
        map.insert(name, totals);
    }
    map
}

pub fn pod_status_time(pod: &Pod) -> Option<DateTime<Utc>> {
    // Prefer status.startTime, fallback to metadata.creationTimestamp
    if let Some(st) = pod.status.as_ref().and_then(|s| s.start_time.as_ref()) {
        return Some(st.0);
    }
    pod.metadata
        .creation_timestamp
        .as_ref()
        .map(|t| t.0)
}
