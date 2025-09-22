use criterion::{black_box, criterion_group, criterion_main, Criterion};
use kube_health_reporter::parsing::{parse_cpu_to_millicores, parse_memory_to_bytes};

fn cpu_parsing_benchmark(c: &mut Criterion) {
    let test_values = vec![
        "100m",
        "1",
        "0.5",
        "2.5",
        "1000000000n",
        "1000000u",
        "500m",
        "1500m",
    ];
    
    c.bench_function("parse_cpu_to_millicores", |b| {
        b.iter(|| {
            for value in &test_values {
                black_box(parse_cpu_to_millicores(black_box(value)));
            }
        })
    });
}

fn memory_parsing_benchmark(c: &mut Criterion) {
    let test_values = vec![
        "1Ki",
        "1Mi", 
        "1Gi",
        "1Ti",
        "1K",
        "1M",
        "1G",
        "1T",
        "512Mi",
        "2.5Gi",
    ];
    
    c.bench_function("parse_memory_to_bytes", |b| {
        b.iter(|| {
            for value in &test_values {
                black_box(parse_memory_to_bytes(black_box(value)));
            }
        })
    });
}

criterion_group!(benches, cpu_parsing_benchmark, memory_parsing_benchmark);
criterion_main!(benches);
