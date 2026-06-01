# micropuffer

`micropuffer` is a self-contained, in-memory turbopuffer-compatible query engine for local testing, mocks, and dashboard development.

It intentionally favors correctness over speed. The crate supports row and column writes, filters, vector/BM25/sparse search, aggregations, namespace workspace operations, schema updates, warm-cache mocks, recall debugging, and explain-query shape support.

## Install

```sh
pnpm add micropuffer
```

```ts
import { Micropuffer } from "micropuffer";

const engine = new Micropuffer();
engine.write("local", JSON.stringify({ upsert_rows: [{ id: 1, vector: [1, 0] }] }));
const response = engine.query("local", JSON.stringify({ rank_by: ["id", "asc"], limit: 10 }));
const httpResponse = engine.queryResponse("local", JSON.stringify({ rank_by: ["id", "asc"], limit: 10 }));
const storeJson = engine.exportStore();
```

Raw methods like `query` and `write` throw string errors through the WASM API. The matching `*Response` methods return JSON strings with `{ status, body }`, which is better for HTTP mocks that need turbopuffer-style error status and body parity.

The npm package ships generated `wasm-bindgen` output from `pkg/`. Generated artifacts are built during `prepack` and in the publish workflow, but are not committed to git.

## Build

```sh
cargo build
pnpm build:wasm
```

## Test

```sh
cargo test
cargo clippy --all --benches --tests --examples --all-features
pnpm exec tsc --noEmit
pnpm test:wasm
pnpm test:live
pnpm test:fuzz
```

Live parity tests require:

```sh
TURBOPUFFER_API_KEY=tpuf_...
TURBOPUFFER_REGION=gcp-us-central1
```

See `docs/api-coverage.md` for current coverage and known gaps.

The pnpm workspace sets `minimumReleaseAge: 10080`, so dependency installs ignore package versions published in the last seven days.

## Benchmark

Run the live head-to-head benchmark with a temporary turbopuffer namespace:

```sh
MICROPUFFER_BENCH_ROWS=100000 pnpm bench:live
```

Useful knobs:

```sh
MICROPUFFER_BENCH_DIMS=32
MICROPUFFER_BENCH_BATCH_SIZE=1000
MICROPUFFER_BENCH_QUERY_RUNS=3
MICROPUFFER_BENCH_KEEP_NAMESPACE=1
```

The benchmark covers row writes, ANN, BM25, sparse vector search, filtered order-by, count aggregation, and grouped count aggregation. It prints a Markdown table plus JSON summaries for copying into issues or release notes.

Run the local stateful WASM benchmark without live API credentials:

```sh
MICROPUFFER_STATEFUL_BENCH_ROWS=100000 pnpm bench:stateful
```

## Contributing

See `CONTRIBUTING.md`. The project uses Conventional Commits and the MIT license.
