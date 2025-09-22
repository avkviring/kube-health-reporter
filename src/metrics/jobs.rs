use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use k8s_openapi::api::batch::v1::{Job, CronJob};
use kube::{api::ListParams, Api, Client};

use crate::types::{Config, FailedJobInfo, MissedCronJobInfo};

/// Analyze failed jobs
pub async fn analyze_failed_jobs(
    client: &Client,
    namespace: &str,
    cfg: &Config,
) -> Result<Vec<FailedJobInfo>> {
    let job_api: Api<Job> = Api::namespaced(client.clone(), namespace);
    let jobs = job_api.list(&ListParams::default()).await?;
    let mut failed_jobs = Vec::new();

    for job in jobs.items {
        let job_name = match job.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };

        if is_job_failed_over_grace(&job, cfg.pending_grace_minutes) {
            let failed_pods = job.status.as_ref()
                .and_then(|s| s.failed)
                .unwrap_or(0);
            
            let (last_failure_time, reason) = extract_job_failure_info(&job);

            failed_jobs.push(FailedJobInfo {
                namespace: namespace.to_string(),
                job: job_name,
                failed_pods,
                last_failure_time,
                reason,
            });
        }
    }

    Ok(failed_jobs)
}

/// Analyze missed CronJobs
pub async fn analyze_missed_cronjobs(
    client: &Client,
    namespace: &str,
    grace_minutes: i64,
) -> Result<Vec<MissedCronJobInfo>> {
    let cronjob_api: Api<CronJob> = Api::namespaced(client.clone(), namespace);
    let cronjobs = cronjob_api.list(&ListParams::default()).await?;
    let mut missed_cronjobs = Vec::new();

    for cronjob in cronjobs.items {
        let cronjob_name = match cronjob.metadata.name.as_ref() {
            Some(n) => n.clone(),
            None => continue,
        };

        if let Some((last_schedule_time, missed_runs)) = extract_missed_runs(&cronjob, grace_minutes) {
            missed_cronjobs.push(MissedCronJobInfo {
                namespace: namespace.to_string(),
                cronjob: cronjob_name,
                last_schedule_time,
                missed_runs,
            });
        }
    }

    Ok(missed_cronjobs)
}

// Helper functions
fn is_job_failed_over_grace(job: &Job, grace_minutes: i64) -> bool {
    // Check if job has failed conditions
    let has_failed_condition = job.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .map(|conditions| {
            conditions.iter().any(|c| c.type_ == "Failed" && c.status == "True")
        })
        .unwrap_or(false);

    if !has_failed_condition {
        return false;
    }

    // Check grace period
    let creation_time = job.metadata.creation_timestamp
        .as_ref()
        .map(|t| t.0)
        .unwrap_or_else(Utc::now);
    
    (Utc::now() - creation_time) > Duration::minutes(grace_minutes)
}

fn extract_job_failure_info(job: &Job) -> (Option<DateTime<Utc>>, Option<String>) {
    let last_failure_time = job.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|c| c.type_ == "Failed")
                .and_then(|c| c.last_transition_time.as_ref())
                .map(|t| t.0)
        });

    let reason = job.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .and_then(|conditions| {
            conditions
                .iter()
                .find(|c| c.type_ == "Failed")
                .and_then(|c| c.reason.clone())
        });

    (last_failure_time, reason)
}

fn extract_missed_runs(cronjob: &CronJob, grace_minutes: i64) -> Option<(DateTime<Utc>, i32)> {
    let last_schedule_time = cronjob.status
        .as_ref()
        .and_then(|s| s.last_schedule_time.as_ref())
        .map(|t| t.0)?;

    // Simple heuristic: if last schedule was more than expected interval + grace, it's missed
    let expected_next_run = last_schedule_time + Duration::minutes(grace_minutes);
    
    if Utc::now() > expected_next_run {
        let missed_runs = ((Utc::now() - expected_next_run).num_minutes() / grace_minutes) as i32 + 1;
        Some((last_schedule_time, missed_runs))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::batch::v1::{JobStatus, JobCondition, CronJobStatus};
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

    #[test]
    fn test_is_job_failed_over_grace() {
        let config = create_test_config();
        let old_time = Utc::now() - Duration::minutes(10);
        let recent_time = Utc::now() - Duration::minutes(2);

        // Test failed job over grace period
        let mut job = Job {
            metadata: ObjectMeta {
                name: Some("test-job".to_string()),
                creation_timestamp: Some(Time(old_time)),
                ..Default::default()
            },
            status: Some(JobStatus {
                conditions: Some(vec![
                    JobCondition {
                        type_: "Failed".to_string(),
                        status: "True".to_string(),
                        last_transition_time: Some(Time(old_time + Duration::minutes(1))),
                        ..Default::default()
                    }
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        assert!(is_job_failed_over_grace(&job, config.pending_grace_minutes));

        // Test failed job within grace period
        job.metadata.creation_timestamp = Some(Time(recent_time));
        assert!(!is_job_failed_over_grace(&job, config.pending_grace_minutes));

        // Test successful job
        job.status.as_mut().unwrap().conditions = Some(vec![
            JobCondition {
                type_: "Complete".to_string(),
                status: "True".to_string(),
                ..Default::default()
            }
        ]);
        assert!(!is_job_failed_over_grace(&job, config.pending_grace_minutes));
    }

    #[test]
    fn test_extract_job_failure_info() {
        let failure_time = Utc::now() - Duration::minutes(5);
        let job = Job {
            metadata: ObjectMeta {
                name: Some("test-job".to_string()),
                ..Default::default()
            },
            status: Some(JobStatus {
                conditions: Some(vec![
                    JobCondition {
                        type_: "Failed".to_string(),
                        status: "True".to_string(),
                        last_transition_time: Some(Time(failure_time)),
                        reason: Some("BackoffLimitExceeded".to_string()),
                        ..Default::default()
                    }
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let (last_failure_time, reason) = extract_job_failure_info(&job);
        assert_eq!(last_failure_time, Some(failure_time));
        assert_eq!(reason, Some("BackoffLimitExceeded".to_string()));
    }

    #[test]
    fn test_extract_missed_runs() {
        let last_schedule = Utc::now() - Duration::minutes(20);
        let grace_minutes = 5;

        let cronjob = CronJob {
            metadata: ObjectMeta {
                name: Some("test-cronjob".to_string()),
                ..Default::default()
            },
            status: Some(CronJobStatus {
                last_schedule_time: Some(Time(last_schedule)),
                ..Default::default()
            }),
            ..Default::default()
        };

        let missed_info = extract_missed_runs(&cronjob, grace_minutes);
        assert!(missed_info.is_some());
        let (schedule_time, missed_runs) = missed_info.unwrap();
        assert_eq!(schedule_time, last_schedule);
        assert!(missed_runs > 0);

        // Test recent schedule (no missed runs)
        let recent_schedule = Utc::now() - Duration::minutes(2);
        let cronjob = CronJob {
            metadata: ObjectMeta {
                name: Some("test-cronjob".to_string()),
                ..Default::default()
            },
            status: Some(CronJobStatus {
                last_schedule_time: Some(Time(recent_schedule)),
                ..Default::default()
            }),
            ..Default::default()
        };

        let missed_info = extract_missed_runs(&cronjob, grace_minutes);
        assert!(missed_info.is_none());
    }
}
