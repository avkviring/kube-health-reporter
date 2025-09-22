use crate::types::{PodUsageTotals, PodRequestTotals};

pub fn parse_cpu_to_millicores(q: &str) -> Option<i64> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    if let Some(stripped) = q.strip_suffix('n') {
        if let Ok(nanos) = stripped.parse::<i128>() {
            return Some((nanos / 1_000_000) as i64);
        }
    } else if let Some(stripped) = q.strip_suffix('u') {
        if let Ok(micros) = stripped.parse::<i128>() {
            return Some((micros / 1_000) as i64);
        }
    } else if let Some(stripped) = q.strip_suffix('m') {
        if let Ok(mc) = stripped.parse::<i64>() {
            return Some(mc);
        }
    } else {
        // treat as cores; can be integer or float
        if let Ok(cores) = q.parse::<f64>() {
            return Some((cores * 1000.0).round() as i64);
        }
    }
    None
}

pub fn parse_memory_to_bytes(q: &str) -> Option<i64> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }

    // Order matters: check binary suffixes first (Ki, Mi, ...), then decimal (K, M, ...)
    const BINARY_UNITS: &[(&str, i64)] = &[
        ("Ki", 1024),
        ("Mi", 1024 * 1024),
        ("Gi", 1024 * 1024 * 1024),
        ("Ti", 1024_i64.pow(4)),
        ("Pi", 1024_i64.pow(5)),
        ("Ei", 1024_i64.pow(6)),
    ];
    const DECIMAL_UNITS: &[(&str, i64)] = &[
        ("K", 1000),
        ("M", 1000 * 1000),
        ("G", 1000 * 1000 * 1000),
        ("T", 1000_i64.pow(4)),
        ("P", 1000_i64.pow(5)),
        ("E", 1000_i64.pow(6)),
        ("k", 1000),
    ];

    for (suf, mul) in BINARY_UNITS {
        if let Some(stripped) = q.strip_suffix(suf) {
            if let Ok(v) = stripped.parse::<f64>() {
                return Some((v * (*mul as f64)).round() as i64);
            }
        }
    }
    for (suf, mul) in DECIMAL_UNITS {
        if let Some(stripped) = q.strip_suffix(suf) {
            if let Ok(v) = stripped.parse::<f64>() {
                return Some((v * (*mul as f64)).round() as i64);
            }
        }
    }
    // bytes without suffix
    if let Ok(v) = q.parse::<i64>() {
        return Some(v);
    }
    None
}

pub fn compute_utilization_percentages(usage: &PodUsageTotals, req: &PodRequestTotals) -> (Option<f64>, Option<f64>) {
    let cpu_pct = match req.cpu_millicores {
        Some(req_mc) if req_mc > 0 => Some((usage.cpu_millicores as f64) / (req_mc as f64) * 100.0),
        _ => None,
    };
    let mem_pct = match req.memory_bytes {
        Some(req_b) if req_b > 0 => Some((usage.memory_bytes as f64) / (req_b as f64) * 100.0),
        _ => None,
    };
    (cpu_pct, mem_pct)
}

pub fn any_exceeds(cpu_pct: Option<f64>, mem_pct: Option<f64>, threshold: f64) -> Option<bool> {
    match (cpu_pct, mem_pct) {
        (None, None) => None,
        (c, m) => Some(c.map(|v| v > threshold).unwrap_or(false) || m.map(|v| v > threshold).unwrap_or(false)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cpu_to_millicores() {
        // Test nanoseconds
        assert_eq!(parse_cpu_to_millicores("1000000000n"), Some(1000));
        assert_eq!(parse_cpu_to_millicores("500000000n"), Some(500));
        
        // Test microseconds
        assert_eq!(parse_cpu_to_millicores("1000000u"), Some(1000));
        assert_eq!(parse_cpu_to_millicores("500000u"), Some(500));
        
        // Test millicores
        assert_eq!(parse_cpu_to_millicores("100m"), Some(100));
        assert_eq!(parse_cpu_to_millicores("1500m"), Some(1500));
        
        // Test cores (as float)
        assert_eq!(parse_cpu_to_millicores("1"), Some(1000));
        assert_eq!(parse_cpu_to_millicores("0.5"), Some(500));
        assert_eq!(parse_cpu_to_millicores("2.5"), Some(2500));
        
        // Test invalid inputs
        assert_eq!(parse_cpu_to_millicores(""), None);
        assert_eq!(parse_cpu_to_millicores("invalid"), None);
        assert_eq!(parse_cpu_to_millicores("100x"), None);
    }

    #[test]
    fn test_parse_memory_to_bytes() {
        // Test binary units
        assert_eq!(parse_memory_to_bytes("1Ki"), Some(1024));
        assert_eq!(parse_memory_to_bytes("1Mi"), Some(1024 * 1024));
        assert_eq!(parse_memory_to_bytes("1Gi"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_to_bytes("2.5Mi"), Some((2.5 * 1024.0 * 1024.0) as i64));
        
        // Test decimal units
        assert_eq!(parse_memory_to_bytes("1K"), Some(1000));
        assert_eq!(parse_memory_to_bytes("1M"), Some(1000 * 1000));
        assert_eq!(parse_memory_to_bytes("1G"), Some(1000 * 1000 * 1000));
        assert_eq!(parse_memory_to_bytes("1k"), Some(1000)); // lowercase k
        
        // Test bytes without suffix
        assert_eq!(parse_memory_to_bytes("1024"), Some(1024));
        assert_eq!(parse_memory_to_bytes("500"), Some(500));
        
        // Test invalid inputs
        assert_eq!(parse_memory_to_bytes(""), None);
        assert_eq!(parse_memory_to_bytes("invalid"), None);
        assert_eq!(parse_memory_to_bytes("100X"), None);
    }

    #[test]
    fn test_compute_utilization_percentages() {
        let usage = PodUsageTotals {
            cpu_millicores: 500,
            memory_bytes: 1024 * 1024 * 512, // 512 MiB
        };
        
        // Test with valid requests
        let requests = PodRequestTotals {
            cpu_millicores: Some(1000), // 1 CPU
            memory_bytes: Some(1024 * 1024 * 1024), // 1 GiB
        };
        
        let (cpu_pct, mem_pct) = compute_utilization_percentages(&usage, &requests);
        assert_eq!(cpu_pct, Some(50.0));
        assert_eq!(mem_pct, Some(50.0));
        
        // Test with no requests
        let no_requests = PodRequestTotals {
            cpu_millicores: None,
            memory_bytes: None,
        };
        
        let (cpu_pct, mem_pct) = compute_utilization_percentages(&usage, &no_requests);
        assert_eq!(cpu_pct, None);
        assert_eq!(mem_pct, None);
        
        // Test with zero requests
        let zero_requests = PodRequestTotals {
            cpu_millicores: Some(0),
            memory_bytes: Some(0),
        };
        
        let (cpu_pct, mem_pct) = compute_utilization_percentages(&usage, &zero_requests);
        assert_eq!(cpu_pct, None);
        assert_eq!(mem_pct, None);
    }

    #[test]
    fn test_any_exceeds() {
        // Test both exceed
        assert_eq!(any_exceeds(Some(90.0), Some(95.0), 85.0), Some(true));
        
        // Test CPU exceeds, memory doesn't
        assert_eq!(any_exceeds(Some(90.0), Some(80.0), 85.0), Some(true));
        
        // Test memory exceeds, CPU doesn't
        assert_eq!(any_exceeds(Some(80.0), Some(90.0), 85.0), Some(true));
        
        // Test neither exceeds
        assert_eq!(any_exceeds(Some(80.0), Some(75.0), 85.0), Some(false));
        
        // Test with None values
        assert_eq!(any_exceeds(None, None, 85.0), None);
        assert_eq!(any_exceeds(Some(90.0), None, 85.0), Some(true));
        assert_eq!(any_exceeds(None, Some(90.0), 85.0), Some(true));
        assert_eq!(any_exceeds(Some(80.0), None, 85.0), Some(false));
    }
}
