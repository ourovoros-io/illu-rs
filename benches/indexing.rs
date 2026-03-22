#![expect(clippy::unwrap_used, reason = "benchmarks")]

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use illu_rs::db::Database;
use illu_rs::indexer::{IndexConfig, index_repo};
use illu_rs::server::tools::QueryScope;

fn bench_self_index(c: &mut Criterion) {
    c.bench_function("index_illu_rs", |b| {
        b.iter(|| {
            let db = Database::open_in_memory().unwrap();
            let config = IndexConfig {
                repo_path: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            };
            index_repo(&db, black_box(&config)).unwrap();
        });
    });
}

fn bench_query_after_index(c: &mut Criterion) {
    let db = Database::open_in_memory().unwrap();
    let config = IndexConfig {
        repo_path: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    };
    index_repo(&db, &config).unwrap();

    let mut group = c.benchmark_group("tools");
    group.bench_function("query_symbol", |b| {
        b.iter(|| {
            illu_rs::server::tools::query::handle_query(
                &db,
                black_box("Database"),
                Some(QueryScope::Symbols),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        });
    });
    group.bench_function("context", |b| {
        b.iter(|| {
            illu_rs::server::tools::context::handle_context(
                &db,
                black_box("Database"),
                false,
                None,
                None,
                None,
                false,
            )
            .unwrap();
        });
    });
    group.bench_function("impact", |b| {
        b.iter(|| {
            illu_rs::server::tools::impact::handle_impact(
                &db,
                black_box("Database"),
                None,
                false,
                false,
            )
            .unwrap();
        });
    });
    group.bench_function("overview", |b| {
        b.iter(|| {
            illu_rs::server::tools::overview::handle_overview(&db, black_box("src/"), false, None)
                .unwrap();
        });
    });
    group.finish();
}

criterion_group!(benches, bench_self_index, bench_query_after_index);
criterion_main!(benches);
