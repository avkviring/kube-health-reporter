use kube_health_reporter::{
    parse_cpu_to_millicores, parse_memory_to_bytes, compute_utilization_percentages,
    any_exceeds, build_slack_payload, load_config_with_env, MockEnvironment, PodUsageTotals, PodRequestTotals,
    HeavyUsagePod, RestartEventInfo, PendingPodInfo, Config
};
use kube_health_reporter::report::{HealthReport, ReportSummary};

#[test]
fn test_cpu_parsing_edge_cases() {
    // Test various edge cases for CPU parsing
    assert_eq!(parse_cpu_to_millicores("0"), Some(0));
    assert_eq!(parse_cpu_to_millicores("0.001"), Some(1));
    assert_eq!(parse_cpu_to_millicores("10.5"), Some(10500));
    
    // Test with whitespace
    assert_eq!(parse_cpu_to_millicores("  100m  "), Some(100));
    assert_eq!(parse_cpu_to_millicores("\t1\n"), Some(1000));
    
    // Test extreme values
    assert_eq!(parse_cpu_to_millicores("999999999n"), Some(999));
    assert_eq!(parse_cpu_to_millicores("1000000u"), Some(1000));
}

#[test]
fn test_memory_parsing_edge_cases() {
    // Test various edge cases for memory parsing
    assert_eq!(parse_memory_to_bytes("0"), Some(0));
    assert_eq!(parse_memory_to_bytes("1"), Some(1));
    
    // Test with whitespace
    assert_eq!(parse_memory_to_bytes("  1Mi  "), Some(1024 * 1024));
    assert_eq!(parse_memory_to_bytes("\t1Gi\n"), Some(1024 * 1024 * 1024));
    
    // Test fractional values
    assert_eq!(parse_memory_to_bytes("0.5Gi"), Some((0.5 * 1024.0 * 1024.0 * 1024.0) as i64));
    assert_eq!(parse_memory_to_bytes("1.5Mi"), Some((1.5 * 1024.0 * 1024.0) as i64));
    
    // Test priority of binary vs decimal units (binary should be checked first)
    assert_eq!(parse_memory_to_bytes("1Ki"), Some(1024));  // Ki should be parsed as binary
    assert_eq!(parse_memory_to_bytes("1K"), Some(1000));   // K should be parsed as decimal
}

#[test]
fn test_utilization_calculations_edge_cases() {
    // Test with zero usage
    let zero_usage = PodUsageTotals {
        cpu_millicores: 0,
        memory_bytes: 0,
    };
    
    let requests = PodRequestTotals {
        cpu_millicores: Some(1000),
        memory_bytes: Some(1024 * 1024 * 1024),
    };
    
    let (cpu_pct, mem_pct) = compute_utilization_percentages(&zero_usage, &requests);
    assert_eq!(cpu_pct, Some(0.0));
    assert_eq!(mem_pct, Some(0.0));
    
    // Test with very high usage (over 100%)
    let high_usage = PodUsageTotals {
        cpu_millicores: 2000, // 200% of request
        memory_bytes: 2 * 1024 * 1024 * 1024, // 200% of request
    };
    
    let (cpu_pct, mem_pct) = compute_utilization_percentages(&high_usage, &requests);
    assert_eq!(cpu_pct, Some(200.0));
    assert_eq!(mem_pct, Some(200.0));
}

#[test]
fn test_threshold_checking_comprehensive() {
    // Test exact threshold match
    assert_eq!(any_exceeds(Some(85.0), Some(85.0), 85.0), Some(false));
    
    // Test just above threshold
    assert_eq!(any_exceeds(Some(85.1), Some(84.9), 85.0), Some(true));
    assert_eq!(any_exceeds(Some(84.9), Some(85.1), 85.0), Some(true));
    
    // Test with very small differences
    assert_eq!(any_exceeds(Some(85.000001), Some(84.999999), 85.0), Some(true));
    
    // Test with zero threshold
    assert_eq!(any_exceeds(Some(0.1), Some(0.0), 0.0), Some(true));
    assert_eq!(any_exceeds(Some(0.0), Some(0.0), 0.0), Some(false));
    
    // Test with negative values (shouldn't happen in practice but good to test)
    assert_eq!(any_exceeds(Some(-10.0), Some(-5.0), 0.0), Some(false));
}

#[test]
fn test_config_environment_isolation() {
    // Test that missing required variables cause errors
    let empty_env = MockEnvironment::new();
    assert!(load_config_with_env(&empty_env).is_err());
    
    // Set minimal config
    let env = MockEnvironment::new()
        .with_var("NAMESPACES", "test-ns1,test-ns2")
        .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/services/test");
    
    let config = load_config_with_env(&env).unwrap();
    assert_eq!(config.namespaces, vec!["test-ns1", "test-ns2"]);
    assert_eq!(config.slack_webhook_url, "https://hooks.slack.com/services/test");
    
    // Test namespace parsing with various formats
    let env = MockEnvironment::new()
        .with_var("NAMESPACES", " ns1 , ns2 ,  ns3  ,")
        .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/services/test");
    
    let config = load_config_with_env(&env).unwrap();
    assert_eq!(config.namespaces, vec!["ns1", "ns2", "ns3"]);
    
    // Test empty namespaces after trimming
    let env = MockEnvironment::new()
        .with_var("NAMESPACES", " , , ,")
        .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/services/test");
    
    assert!(load_config_with_env(&env).is_err());
}

#[test]
fn test_slack_payload_formatting() {
    let config = Config {
        namespaces: vec!["prod".to_string(), "staging".to_string()],
        threshold_percent: 90.0,
        slack_webhook_url: "https://hooks.slack.com/test".to_string(),
        restart_grace_minutes: 3,
        pending_grace_minutes: 7,
        cluster_name: Some("production-cluster".to_string()),
        datacenter_name: Some("eu-west-1".to_string()),
        fail_if_no_metrics: false,
    };
    
    // Test with multiple items of each type
    let heavy_usage = vec![
        HeavyUsagePod {
            namespace: "prod".to_string(),
            pod: "api-server-1".to_string(),
            cpu_pct: Some(95.5),
            mem_pct: Some(87.2),
        },
        HeavyUsagePod {
            namespace: "staging".to_string(),
            pod: "worker-2".to_string(),
            cpu_pct: None, // Only memory exceeds
            mem_pct: Some(92.8),
        },
    ];
    
    let restarts = vec![
        RestartEventInfo {
            namespace: "prod".to_string(),
            pod: "database-1".to_string(),
            container: "postgres".to_string(),
            last_restart_time: Some(chrono::Utc::now()),
            reason: Some("OOMKilled".to_string()),
            message: Some("Container exceeded memory limit".to_string()),
            exit_code: Some(137),
        },
    ];
    
    let pendings = vec![
        PendingPodInfo {
            namespace: "staging".to_string(),
            pod: "new-deployment".to_string(),
            since: chrono::Utc::now() - chrono::Duration::minutes(15),
            duration_minutes: 15,
        },
    ];
    
    let payload = build_slack_payload(&config, &heavy_usage, &restarts, &pendings, &[], &[], &[], &[], &[], &[], &[], &[]);
    
    // Verify structure - now has 13 blocks (header + config + 11 metric sections)
    assert_eq!(payload.blocks.len(), 13);
    assert!(payload.text.is_none());
    
    // Check header contains cluster name and datacenter name
    let header_text = payload.blocks[0]["text"]["text"].as_str().unwrap();
    assert!(header_text.contains("production-cluster"));
    assert!(header_text.contains("eu-west-1"));
    
    // Check config section contains all settings
    let config_text = payload.blocks[1]["text"]["text"].as_str().unwrap();
    assert!(config_text.contains("prod, staging"));
    assert!(config_text.contains("90%"));
    assert!(config_text.contains("restarts 3m"));
    assert!(config_text.contains("pending 7m"));
    
    // Check heavy usage section
    let heavy_text = payload.blocks[2]["text"]["text"].as_str().unwrap();
    assert!(heavy_text.contains("prod/api-server-1"));
    assert!(heavy_text.contains("96%")); // Rounded from 95.5
    assert!(heavy_text.contains("87%")); // Rounded from 87.2
    assert!(heavy_text.contains("staging/worker-2"));
    assert!(heavy_text.contains("-")); // For missing CPU percentage
    assert!(heavy_text.contains("93%")); // Rounded from 92.8
    
    // Check restarts section
    let restart_text = payload.blocks[3]["text"]["text"].as_str().unwrap();
    assert!(restart_text.contains("prod/database-1"));
    assert!(restart_text.contains("[postgres]"));
    assert!(restart_text.contains("OOMKilled"));
    assert!(restart_text.contains("(exit 137)"));
    assert!(restart_text.contains("Container exceeded memory limit"));
    
    // Check pending section
    let pending_text = payload.blocks[4]["text"]["text"].as_str().unwrap();
    assert!(pending_text.contains("staging/new-deployment"));
    assert!(pending_text.contains("pending for 15m"));
}

#[test]
fn test_boolean_config_parsing() {
    // Test various truthy values
    for val in ["1", "true", "TRUE", "True"] {
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", "test")
            .with_var("SLACK_WEBHOOK_URL", "https://test.com")
            .with_var("FAIL_IF_NO_METRICS", val);
        
        let config = load_config_with_env(&env).unwrap();
        assert!(config.fail_if_no_metrics, "Failed for value: {}", val);
    }
    
    // Test various falsy values
    for val in ["0", "false", "FALSE", "False", "no", "off", ""] {
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", "test")
            .with_var("SLACK_WEBHOOK_URL", "https://test.com")
            .with_var("FAIL_IF_NO_METRICS", val);
        
        let config = load_config_with_env(&env).unwrap();
        assert!(!config.fail_if_no_metrics, "Failed for value: {}", val);
    }
    
    // Test missing value (should default to true)
    let env = MockEnvironment::new()
        .with_var("NAMESPACES", "test")
        .with_var("SLACK_WEBHOOK_URL", "https://test.com");
    
    let config = load_config_with_env(&env).unwrap();
    assert!(config.fail_if_no_metrics);
}

#[test]
fn test_report_summary_has_issues() {
    // Test ReportSummary with no issues
    let empty_summary = ReportSummary {
        heavy_usage_count: 0,
        restart_count: 0,
        pending_count: 0,
        failed_pod_count: 0,
        unready_count: 0,
        oom_killed_count: 0,
        failed_job_count: 0,
        missed_cronjob_count: 0,
        volume_issue_count: 0,
        problematic_node_count: 0,
        high_util_node_count: 0,
    };
    
    assert_eq!(empty_summary.total_issues(), 0);
    assert!(!empty_summary.has_issues());
    
    // Test ReportSummary with issues
    let summary_with_issues = ReportSummary {
        heavy_usage_count: 2,
        restart_count: 1,
        pending_count: 0,
        failed_pod_count: 1,
        unready_count: 0,
        oom_killed_count: 1,
        failed_job_count: 0,
        missed_cronjob_count: 0,
        volume_issue_count: 0,
        problematic_node_count: 1,
        high_util_node_count: 0,
    };
    
    assert_eq!(summary_with_issues.total_issues(), 6);
    assert!(summary_with_issues.has_issues());
    
    // Test ReportSummary with just one issue
    let single_issue_summary = ReportSummary {
        heavy_usage_count: 0,
        restart_count: 0,
        pending_count: 0,
        failed_pod_count: 0,
        unready_count: 0,
        oom_killed_count: 0,
        failed_job_count: 0,
        missed_cronjob_count: 0,
        volume_issue_count: 1,
        problematic_node_count: 0,
        high_util_node_count: 0,
    };
    
    assert_eq!(single_issue_summary.total_issues(), 1);
    assert!(single_issue_summary.has_issues());
}

#[test]
fn test_health_report_has_issues() {
    let config = Config {
        namespaces: vec!["test".to_string()],
        threshold_percent: 85.0,
        slack_webhook_url: "https://hooks.slack.com/test".to_string(),
        restart_grace_minutes: 5,
        pending_grace_minutes: 5,
        cluster_name: None,
        datacenter_name: None,
        fail_if_no_metrics: true,
    };
    
    // Test empty report
    let empty_report = HealthReport::new(config.clone());
    assert!(!empty_report.has_issues());
    
    // Test report with heavy usage pod
    let mut report_with_issues = HealthReport::new(config.clone());
    report_with_issues.pod_metrics.heavy_usage.push(HeavyUsagePod {
        namespace: "test".to_string(),
        pod: "heavy-pod".to_string(),
        cpu_pct: Some(90.0),
        mem_pct: Some(95.0),
    });
    
    assert!(report_with_issues.has_issues());
    let summary = report_with_issues.summary();
    assert_eq!(summary.total_issues(), 1);
    assert!(summary.has_issues());
}
