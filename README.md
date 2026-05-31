# micropuffer

`micropuffer` is a self-contained, in-memory turbopuffer-compatible query engine for local testing, mocks, and dashboard development.

It intentionally favors correctness over speed. The crate supports row and column writes, filters, vector/BM25/sparse search, aggregations, namespace workspace operations, schema updates, warm-cache mocks, recall debugging, and explain-query shape support.

## Build

```sh
cargo build
npm run build:wasm
```

## Test

```sh
cargo test
cargo clippy --all --benches --tests --examples --all-features
npm run test:live
npm run test:fuzz
```

Live parity tests require:

```sh
TURBOPUFFER_API_KEY=tpuf_...
TURBOPUFFER_REGION=gcp-us-central1
```

See `docs/api-coverage.md` for current coverage and known gaps.
