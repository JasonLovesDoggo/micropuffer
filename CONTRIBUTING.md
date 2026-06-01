# Contributing

Thanks for working on `micropuffer`. This project is meant to be a boringly correct local stand-in for turbopuffer, so correctness and parity evidence matter more than cleverness.

## Development

Use pnpm and Rust stable.

```sh
pnpm install
just check
just wasm
```

The pnpm workspace uses `minimumReleaseAge: 10080`, which ignores package versions published in the last seven days. Keep that security gate enabled.

Generated `wasm-bindgen` output lives in `pkg/`. It is built by `just wasm` and `prepack`, but it is not committed.

## Tests

Run the Rust suite before sending changes:

```sh
just check
```

Live parity tests require a turbopuffer API key:

```sh
TURBOPUFFER_API_KEY=tpuf_...
TURBOPUFFER_REGION=gcp-us-central1
just test-live
just test-fuzz
```

For benchmark work, use a temporary namespace and clean it up:

```sh
MICROPUFFER_BENCH_ROWS=100000 just bench-live
```

## Commits

Use Conventional Commits:

```text
feat: add sparse query parity case
fix: match live datetime projection
test: add live filter fuzz case
bench: add 100k vector benchmark harness
docs: explain npm publish flow
ci: add npm provenance publish workflow
chore: update generated lockfile
```

Prefer small, reviewable commits. If a change affects live parity, include the exact command you ran in the PR or handoff.

## Pull Requests

Good PRs include:

- What changed.
- Which turbopuffer behavior or docs were used as the source of truth.
- Commands run.
- Any live API gaps, skipped cases, or account-permission blockers.

Be kind in review. Assume the person on the other side is trying to make the local clone less wrong.
