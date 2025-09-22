use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use crate::types::Config;

/// Trait for abstracting environment variable access
pub trait EnvironmentProvider {
    fn get_var(&self, key: &str) -> Option<String>;
}

/// Production implementation using std::env
pub struct SystemEnvironment;

impl EnvironmentProvider for SystemEnvironment {
    fn get_var(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

/// Mock implementation for testing
#[derive(Debug, Default)]
pub struct MockEnvironment {
    vars: HashMap<String, String>,
}

impl MockEnvironment {
    pub fn new() -> Self {
        Self {
            vars: HashMap::new(),
        }
    }
    
    pub fn set_var<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.vars.insert(key.into(), value.into());
        self
    }
    
    pub fn with_var<K, V>(mut self, key: K, value: V) -> Self
    where
        K: Into<String>,
        V: Into<String>,
    {
        self.set_var(key, value);
        self
    }
}

impl EnvironmentProvider for MockEnvironment {
    fn get_var(&self, key: &str) -> Option<String> {
        self.vars.get(key).cloned()
    }
}

pub fn load_config() -> Result<Config> {
    load_config_with_env(&SystemEnvironment)
}

pub fn load_config_with_env<E: EnvironmentProvider>(env: &E) -> Result<Config> {
    let namespaces = env.get_var("NAMESPACES").unwrap_or_default();
    let namespaces: Vec<String> = namespaces
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if namespaces.is_empty() {
        return Err(anyhow!("NAMESPACES env var must be set (comma-separated)"));
    }

    let threshold_percent: f64 = env.get_var("THRESHOLD_PERCENT")
        .unwrap_or_else(|| "85".to_string())
        .parse()
        .context("Invalid THRESHOLD_PERCENT")?;

    let slack_webhook_url = env.get_var("SLACK_WEBHOOK_URL")
        .ok_or_else(|| anyhow!("SLACK_WEBHOOK_URL must be provided via Secret env"))?;

    let restart_grace_minutes: i64 = env.get_var("RESTART_GRACE_MINUTES")
        .unwrap_or_else(|| "5".to_string())
        .parse()
        .unwrap_or(5);

    let pending_grace_minutes: i64 = env.get_var("PENDING_GRACE_MINUTES")
        .unwrap_or_else(|| "5".to_string())
        .parse()
        .unwrap_or(5);

    let cluster_name = env.get_var("CLUSTER_NAME");
    let datacenter_name = env.get_var("DATACENTER_NAME");

    let fail_if_no_metrics = env.get_var("FAIL_IF_NO_METRICS")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "True"))
        .unwrap_or(true); // default to true per requirement

    Ok(Config {
        namespaces,
        threshold_percent,
        slack_webhook_url,
        restart_grace_minutes,
        pending_grace_minutes,
        cluster_name,
        datacenter_name,
        fail_if_no_metrics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loading_with_env() {
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", "default,kube-system,monitoring")
            .with_var("THRESHOLD_PERCENT", "90")
            .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/test")
            .with_var("RESTART_GRACE_MINUTES", "10")
            .with_var("PENDING_GRACE_MINUTES", "15")
            .with_var("CLUSTER_NAME", "test-cluster")
            .with_var("DATACENTER_NAME", "us-west-1")
            .with_var("FAIL_IF_NO_METRICS", "false");
        
        let config = load_config_with_env(&env).unwrap();
        
        assert_eq!(config.namespaces, vec!["default", "kube-system", "monitoring"]);
        assert_eq!(config.threshold_percent, 90.0);
        assert_eq!(config.slack_webhook_url, "https://hooks.slack.com/test");
        assert_eq!(config.restart_grace_minutes, 10);
        assert_eq!(config.pending_grace_minutes, 15);
        assert_eq!(config.cluster_name, Some("test-cluster".to_string()));
        assert_eq!(config.datacenter_name, Some("us-west-1".to_string()));
        assert_eq!(config.fail_if_no_metrics, false);
    }

    #[test]
    fn test_config_loading_defaults() {
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", "default")
            .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/test");
        
        let config = load_config_with_env(&env).unwrap();
        
        assert_eq!(config.namespaces, vec!["default"]);
        assert_eq!(config.threshold_percent, 85.0); // default
        assert_eq!(config.restart_grace_minutes, 5); // default
        assert_eq!(config.pending_grace_minutes, 5); // default
        assert_eq!(config.cluster_name, None); // default
        assert_eq!(config.datacenter_name, None); // default
        assert_eq!(config.fail_if_no_metrics, true); // default
    }

    #[test]
    fn test_config_loading_missing_required() {
        // Test missing NAMESPACES
        let env = MockEnvironment::new()
            .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/test");
        
        let result = load_config_with_env(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("NAMESPACES"));
        
        // Test missing SLACK_WEBHOOK_URL
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", "default");
        
        let result = load_config_with_env(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("SLACK_WEBHOOK_URL"));
    }

    #[test]
    fn test_config_loading_invalid_threshold() {
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", "default")
            .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/test")
            .with_var("THRESHOLD_PERCENT", "invalid");
        
        let result = load_config_with_env(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("THRESHOLD_PERCENT"));
    }

    #[test]
    fn test_namespace_parsing() {
        // Test various namespace formats
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", " ns1 , ns2 ,  ns3  ,")
            .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/test");
        
        let config = load_config_with_env(&env).unwrap();
        assert_eq!(config.namespaces, vec!["ns1", "ns2", "ns3"]);
        
        // Test empty namespaces after trimming
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", " , , ,")
            .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/test");
        
        let result = load_config_with_env(&env);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("NAMESPACES"));
    }

    #[test]
    fn test_boolean_parsing() {
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
    fn test_numeric_parsing_with_invalid_values() {
        // Test invalid grace minutes (should use defaults)
        let env = MockEnvironment::new()
            .with_var("NAMESPACES", "default")
            .with_var("SLACK_WEBHOOK_URL", "https://hooks.slack.com/test")
            .with_var("RESTART_GRACE_MINUTES", "invalid")
            .with_var("PENDING_GRACE_MINUTES", "also_invalid");
        
        let config = load_config_with_env(&env).unwrap();
        assert_eq!(config.restart_grace_minutes, 5); // default fallback
        assert_eq!(config.pending_grace_minutes, 5); // default fallback
    }
}
