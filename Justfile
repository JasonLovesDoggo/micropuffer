set dotenv-load := true

default:
    just --list

fmt:
    cargo fmt --check

fmt-fix:
    cargo fmt

clippy:
    cargo clippy --all --benches --tests --examples --all-features -- -D warnings

test:
    cargo test

typecheck: wasm
    pnpm exec tsc --noEmit

check: fmt clippy test typecheck

wasm-target:
    rustup target add wasm32-unknown-unknown

wasm: wasm-target
    cargo build --lib --target wasm32-unknown-unknown --release
    wasm-bindgen --target bundler --out-dir pkg --out-name micropuffer target/wasm32-unknown-unknown/release/micropuffer.wasm

clean-generated:
    rm -rf pkg

test-live: wasm
    pnpm exec vitest run scripts/micropuffer-live-parity.test.ts

test-fuzz: wasm
    pnpm exec vitest run scripts/micropuffer-live-fuzz.test.ts

bench-live: wasm
    node --experimental-strip-types scripts/micropuffer-live-bench.ts
