use catalog_bench::{BenchmarkConfig, run_benchmark};

#[test]
fn ten_thousand_asset_smoke_report_has_all_measurements() {
    let report = run_benchmark(BenchmarkConfig {
        assets: 10_000,
        batch_size: 500,
    })
    .unwrap();

    assert_eq!(report.assets, 10_000);
    assert!(!report.sqlite_version.is_empty());
    assert!(report.database_bytes > 0);
    assert!(report.insert_ms > 0.0);
    assert!(report.first_page_ms >= 0.0);
    assert_eq!(report.first_page_rows, 100);
    assert!(report.unavailable_count_ms >= 0.0);
    assert!(report.eviction_plan_ms >= 0.0);

    let value = serde_json::to_value(report).unwrap();
    for key in [
        "assets",
        "sqlite_version",
        "database_bytes",
        "insert_ms",
        "first_page_ms",
        "first_page_rows",
        "unavailable_count_ms",
        "eviction_plan_ms",
    ] {
        assert!(value.get(key).is_some(), "missing report key {key}");
    }
}
