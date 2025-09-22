# kube-health-reporter

Rust-based Kubernetes CronJob that reports pod health to Slack.

## Features
- Reports pods exceeding CPU/Memory requests threshold (% of requests)
- Lists containers with restarts (ignores restarts within first N minutes from pod start)
- Flags pods Pending longer than N minutes
- Sends a single Slack message with sections per finding

## Requirements
- Kubernetes cluster with metrics-server installed
- Slack Incoming Webhook URL stored in a Secret

## Configuration (Helm values)
- `namespaces`: list of target namespaces
- `thresholdPercent`: default 85
- `restartGraceMinutes`: default 5
- `pendingGraceMinutes`: default 5
- `failIfNoMetrics`: default true
- `clusterName`: optional label in Slack header
- `slack.webhookSecretName`: Secret name containing the webhook
- `slack.webhookSecretKey`: Secret key, default `webhook`
- `slack.createSecret`: set true to create Secret from `slack.webhook`
- `slack.webhook`: webhook value used when `createSecret=true`

## Install
```bash
# Build and push image (set registry/tag)
cargo build --release
# Update values.yaml with your image and namespaces
helm upgrade --install kube-health-reporter ./helm/kube-health-reporter \
  --namespace monitoring --create-namespace \
  --set image.repository=YOUR_REGISTRY/kube-health-reporter \
  --set image.tag=0.1.0 \
  --set namespaces="{prod-a,prod-b}" \
  --set slack.webhookSecretName=kube-health-reporter-slack \
  --set slack.webhookSecretKey=webhook
```

If you prefer Helm to create the Secret:
```bash
helm upgrade --install kube-health-reporter ./helm/kube-health-reporter \
  --namespace monitoring --create-namespace \
  --set slack.createSecret=true \
  --set slack.webhook="https://hooks.slack.com/services/..."
```

The CronJob runs every 20 minutes by default.

## Environment variables
- `NAMESPACES` (comma-separated)
- `THRESHOLD_PERCENT`
- `RESTART_GRACE_MINUTES`
- `PENDING_GRACE_MINUTES`
- `FAIL_IF_NO_METRICS` (true/false)
- `CLUSTER_NAME` (optional)
- `SLACK_WEBHOOK_URL` (from Secret)

## RBAC
The chart creates a ClusterRole for reading Pods and metrics, and RoleBindings to each configured namespace.
# kube-health-reporter
