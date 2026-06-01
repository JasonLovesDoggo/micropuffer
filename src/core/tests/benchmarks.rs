use crate::core::tests::*;

#[test]
#[ignore = "performance evidence; run explicitly"]
fn write_100k_new_upserts_completes_in_one_batch() {
    let mut rows = Vec::with_capacity(100_000);
    for id in 0..100_000_u64 {
        let mut row = Map::new();
        row.insert("id".to_string(), Value::Number(Number::from(id)));
        row.insert("vector".to_string(), json!([0.0, 1.0]));
        row.insert("score".to_string(), Value::Number(Number::from(id)));
        rows.push(Value::Object(row));
    }
    let mut store = MiniStore::default();
    let started = std::time::Instant::now();
    let response = write_store(
        &mut store,
        "bulk-upsert",
        &json!({
            "distance_metric": "cosine_distance",
            "upsert_rows": rows
        }),
    )
    .unwrap();
    let elapsed = started.elapsed();

    println!("100k new upserts elapsed: {elapsed:?}");
    assert_eq!(response["rows_upserted"], 100_000);
    assert_eq!(
        store.namespace("bulk-upsert").unwrap().documents.len(),
        100_000
    );
}

#[test]
#[ignore = "performance evidence; run explicitly"]
fn stateful_100k_dense_sparse_query_benchmark() {
    let row_count = benchmark_env_usize("MICROPUFFER_BENCH_ROWS").unwrap_or(100_000);
    let dimensions = benchmark_env_usize("MICROPUFFER_BENCH_DIMS").unwrap_or(32);
    let runs = benchmark_env_usize("MICROPUFFER_BENCH_QUERY_RUNS").unwrap_or(5);
    let rows = (0..row_count)
        .map(|index| {
            json!({
                "id": index as u64,
                "vector": benchmark_vector(index, dimensions),
                "sparse_vector": {
                    format!("{}", index % 128): ((index % 10) + 1) as f64 / 10.0,
                    format!("{}", (index * 7) % 128): ((index % 7) + 1) as f64 / 10.0
                },
                "category": format!("category_{}", index % 10),
                "score": index % 1_000
            })
        })
        .collect::<Vec<_>>();
    let mut clone = Micropuffer::new();
    let write_started = std::time::Instant::now();
    clone
        .write(
            "stateful-100k",
            &json!({
                "distance_metric": "cosine_distance",
                "schema": {
                    "vector": {
                        "type": format!("[{dimensions}]f32"),
                        "ann": true
                    },
                    "sparse_vector": {
                        "type": "{}f16",
                        "sparse_knn": {"distance_metric": "dot_product"}
                    },
                    "category": "string",
                    "score": "uint"
                },
                "upsert_rows": rows
            }),
        )
        .unwrap();
    let write_elapsed = write_started.elapsed();

    let dense_query = json!({
        "rank_by": ["vector", "ANN", benchmark_vector(42, dimensions)],
        "limit": 10,
        "include_attributes": ["category", "score"]
    });
    let sparse_query = json!({
        "rank_by": ["sparse_vector", "SparseKNN", {"7": 0.7, "12": 0.2}],
        "limit": 10,
        "include_attributes": ["category"]
    });
    let filter_order_query = json!({
        "rank_by": ["score", "desc"],
        "filters": ["category", "Eq", "category_7"],
        "limit": 100,
        "include_attributes": ["category", "score"]
    });
    let aggregate_count_query = json!({
        "aggregate_by": {"count": ["Count"]},
        "filters": ["category", "Eq", "category_7"]
    });
    let group_count_query = json!({
        "aggregate_by": {"count": ["Count"]},
        "group_by": ["category"],
        "top_k": 10
    });
    let dense_elapsed = benchmark_query_runs(&clone, "stateful-100k", &dense_query, runs);
    let sparse_elapsed = benchmark_query_runs(&clone, "stateful-100k", &sparse_query, runs);
    let filter_order_elapsed =
        benchmark_query_runs(&clone, "stateful-100k", &filter_order_query, runs);
    let aggregate_count_elapsed =
        benchmark_query_runs(&clone, "stateful-100k", &aggregate_count_query, runs);
    let group_count_elapsed =
        benchmark_query_runs(&clone, "stateful-100k", &group_count_query, runs);

    println!(
        "stateful_100k_dense_sparse_query_benchmark rows={row_count} dimensions={dimensions} runs={runs} write_ms={:.2} dense_total_ms={:.2} dense_mean_ms={:.2} sparse_total_ms={:.2} sparse_mean_ms={:.2} filter_order_total_ms={:.2} filter_order_mean_ms={:.2} aggregate_count_total_ms={:.2} aggregate_count_mean_ms={:.2} group_count_total_ms={:.2} group_count_mean_ms={:.2}",
        write_elapsed.as_secs_f64() * 1_000.0,
        dense_elapsed.as_secs_f64() * 1_000.0,
        dense_elapsed.as_secs_f64() * 1_000.0 / runs as f64,
        sparse_elapsed.as_secs_f64() * 1_000.0,
        sparse_elapsed.as_secs_f64() * 1_000.0 / runs as f64,
        filter_order_elapsed.as_secs_f64() * 1_000.0,
        filter_order_elapsed.as_secs_f64() * 1_000.0 / runs as f64,
        aggregate_count_elapsed.as_secs_f64() * 1_000.0,
        aggregate_count_elapsed.as_secs_f64() * 1_000.0 / runs as f64,
        group_count_elapsed.as_secs_f64() * 1_000.0,
        group_count_elapsed.as_secs_f64() * 1_000.0 / runs as f64
    );
}

fn benchmark_query_runs(
    clone: &Micropuffer,
    namespace_name: &str,
    query: &Value,
    runs: usize,
) -> std::time::Duration {
    let started = std::time::Instant::now();
    for _ in 0..runs {
        std::hint::black_box(clone.query(namespace_name, query).unwrap());
    }
    started.elapsed()
}

fn benchmark_env_usize(key: &str) -> Option<usize> {
    std::env::var(key).ok()?.parse().ok()
}

fn benchmark_vector(seed: usize, dimensions: usize) -> Vec<f64> {
    let mut values = (0..dimensions)
        .map(|index| {
            let raw = ((seed * 31 + index * 17) % 1_000) as f64 / 1_000.0;
            raw * 2.0 - 1.0
        })
        .collect::<Vec<_>>();
    let norm = values.iter().map(|value| value * value).sum::<f64>().sqrt();
    for value in &mut values {
        *value /= norm;
    }
    values
}

#[test]
#[ignore = "performance evidence; run explicitly"]
fn bm25_100k_indexed_query_benchmark() {
    let text_buckets = [
        "walrus arctic mammal",
        "reef coral fish",
        "falcon sky bird",
        "forest fox mammal",
    ];
    let rows = (1..=100_000_u64)
        .map(|id| {
            json!({
                "id": id,
                "text": format!("{} document {id}", text_buckets[id as usize % text_buckets.len()]),
                "category": format!("category_{}", id % 10)
            })
        })
        .collect::<Vec<_>>();
    let mut store = MiniStore::default();
    write_store(
        &mut store,
        "bm25-bench",
        &json!({
            "schema": {
                "text": {
                    "type": "string",
                    "full_text_search": true
                }
            },
            "upsert_rows": rows
        }),
    )
    .unwrap();
    let request = json!({
        "rank_by": ["text", "BM25", "walrus mammal"],
        "limit": 10
    });

    let cold_started = std::time::Instant::now();
    query_store(&store, "bm25-bench", &request).unwrap();
    let cold_elapsed = cold_started.elapsed();

    let warm_runs = 10;
    let warm_started = std::time::Instant::now();
    for _ in 0..warm_runs {
        std::hint::black_box(query_store(&store, "bm25-bench", &request).unwrap());
    }
    let warm_elapsed = warm_started.elapsed();
    println!(
        "bm25_100k_indexed_query_benchmark cold_ms={:.3} warm_runs={warm_runs} warm_total_ms={:.3} warm_mean_ms={:.3}",
        cold_elapsed.as_secs_f64() * 1_000.0,
        warm_elapsed.as_secs_f64() * 1_000.0,
        warm_elapsed.as_secs_f64() * 1_000.0 / warm_runs as f64
    );
}
