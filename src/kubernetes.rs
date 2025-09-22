use anyhow::{anyhow, Result};
use kube::Client;

use crate::types::{Config, HeavyUsagePod, RestartEventInfo, PendingPodInfo};
use crate::metrics::{analyze_heavy_usage, analyze_restarts, analyze_pending_pods, list_pod_metrics_http};

pub async fn ensure_metrics_available(client: &Client, namespaces: &[String]) -> Result<()> {
    let ns = namespaces.get(0).ok_or_else(|| anyhow!("No namespaces provided"))?;
    let _ = list_pod_metrics_http(client, ns).await?;
    Ok(())
}

pub async fn analyze_namespace(
    client: &Client,
    namespace: &str,
    cfg: &Config,
) -> Result<(Vec<HeavyUsagePod>, Vec<RestartEventInfo>, Vec<PendingPodInfo>)> {
    let heavy = analyze_heavy_usage(client, namespace, cfg).await?;
    let restarts = analyze_restarts(client, namespace, cfg).await?;
    let pendings = analyze_pending_pods(client, namespace, cfg).await?;
    
    Ok((heavy, restarts, pendings))
}

