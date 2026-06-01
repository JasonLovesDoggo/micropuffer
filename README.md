# micropuffer

`micropuffer` is a self-contained, in-memory turbopuffer-compatible query engine for local testing, mocks, and dashboard development.

It intentionally favors correctness over speed. The crate supports row and column writes, filters, vector/BM25/sparse search, aggregations, namespace workspace operations, schema updates, warm-cache mocks, recall debugging, and explain-query shape support.

## Install

```sh
pnpm add micropuffer
```

```ts
import { micropuffer_query, micropuffer_write } from "micropuffer";
```

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
