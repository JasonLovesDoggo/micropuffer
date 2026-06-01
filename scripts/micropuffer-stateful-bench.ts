import { performance } from "node:perf_hooks";
import {
  type JsonArray,
  type JsonObject,
  type JsonValue,
  envOrDefault,
  integerEnv,
  parseJsonObject
} from "./test-utils.ts";
import {
  Micropuffer
} from "micropuffer";

type EngineName = "json_roundtrip" | "stateful";

type BenchSample = {
  operation: string;
  engine: EngineName;
  durationMs: number;
};

type BenchSummary = {
  operation: string;
  engine: EngineName;
  runs: number;
  meanMs: number;
  p50Ms: number;
  p95Ms: number;
  minMs: number;
  maxMs: number;
  speedupVsJsonRoundtrip?: number;
};

type QueryCase = {
  operation: string;
  request: JsonObject;
};

const namespaceName = envOrDefault(
  "MICROPUFFER_STATEFUL_BENCH_NAMESPACE",
  `micropuffer-stateful-bench-${Date.now()}-${process.pid}`
);
const rowCount = integerEnv("MICROPUFFER_STATEFUL_BENCH_ROWS", 100_000);
const dimensions = integerEnv("MICROPUFFER_STATEFUL_BENCH_DIMS", 32);
const queryRuns = integerEnv("MICROPUFFER_STATEFUL_BENCH_QUERY_RUNS", 10);

let checksum = 0;

main();

function main(): void {
  validateConfig();
  const engine = seedEngine();
  const storeJson = engine.exportStore();
  const roundtripStore = parseJsonObject(storeJson, "exported store");
  const samples: BenchSample[] = [];

  for (const queryCase of queryCases()) {
    const requestJson = json(queryCase.request);
    assertSameResponse(roundtripStore, engine, queryCase.operation, requestJson);
    for (let run = 0; run < queryRuns; run += 1) {
      samples.push(
        timeSync(queryCase.operation, "json_roundtrip", () =>
          Micropuffer.fromStore(JSON.stringify(roundtripStore)).query(
            namespaceName,
            requestJson
          )
        )
      );
      samples.push(
        timeSync(queryCase.operation, "stateful", () =>
          engine.query(namespaceName, requestJson)
        )
      );
    }
  }

  printSummaries(summarize(samples), storeJson.length);
}

function validateConfig(): void {
  if (rowCount <= 0 || dimensions <= 0 || queryRuns <= 0) {
    throw new Error("stateful benchmark row count, dimensions, and query runs must be positive.");
  }
}

function seedEngine(): Micropuffer {
  const engine = new Micropuffer();
  engine.write(
    namespaceName,
    json({
      distance_metric: "cosine_distance",
      schema: {
        text: { type: "string", full_text_search: true },
        tags: "[]string",
        sparse_vector: {
          type: "{}f16",
          sparse_knn: { distance_metric: "dot_product" }
        }
      },
      upsert_rows: Array.from({ length: rowCount }, (_, index) => row(index + 1))
    })
  );
  return engine;
}

function row(id: number): JsonObject {
  const category = `category_${id % 10}`;
  const textBuckets = [
    "walrus arctic mammal",
    "reef coral fish",
    "falcon sky bird",
    "forest fox mammal"
  ];
  return {
    id,
    vector: vectorFor(id),
    sparse_vector: sparseVectorFor(id),
    category,
    public: id % 2,
    score: id % 1_000,
    text: `${textBuckets[id % textBuckets.length]} document ${id}`,
    tags: [category, id % 2 === 0 ? "even" : "odd"]
  };
}

function vectorFor(id: number): JsonArray {
  const values = Array.from({ length: dimensions }, (_, index) => {
    const raw = ((id * 31 + index * 17) % 1_000) / 1_000;
    return raw * 2 - 1;
  });
  const norm = Math.sqrt(values.reduce((sum, value) => sum + value * value, 0));
  return values.map((value) => value / norm);
}

function sparseVectorFor(id: number): JsonObject {
  return {
    [`${id % 64}`]: ((id % 10) + 1) / 10,
    [`${(id * 7) % 64}`]: ((id % 7) + 1) / 10
  };
}

function queryCases(): QueryCase[] {
  return [
    {
      operation: "ann_top10",
      request: {
        rank_by: ["vector", "ANN", vectorFor(42)],
        limit: 10,
        include_attributes: ["category", "score"]
      }
    },
    {
      operation: "bm25_top10",
      request: {
        rank_by: ["text", "BM25", "walrus mammal"],
        limit: 10,
        include_attributes: ["category"]
      }
    },
    {
      operation: "filter_order_top100",
      request: {
        rank_by: ["score", "desc"],
        filters: ["category", "Eq", "category_7"],
        limit: 100,
        include_attributes: ["category", "score"]
      }
    }
  ];
}

function assertSameResponse(
  roundtripStore: JsonObject,
  engine: Micropuffer,
  operation: string,
  requestJson: string
): void {
  const jsonRoundtrip = parseJsonObject(
    Micropuffer.fromStore(JSON.stringify(roundtripStore)).query(namespaceName, requestJson),
    `${operation} JSON roundtrip response`
  );
  const stateful = parseJsonObject(
    engine.query(namespaceName, requestJson),
    `${operation} stateful response`
  );
  if (JSON.stringify(jsonRoundtrip) !== JSON.stringify(stateful)) {
    throw new Error(`${operation} stateful response differed from JSON roundtrip response.`);
  }
}

function timeSync(
  operation: string,
  engine: EngineName,
  callback: () => string
): BenchSample {
  const start = performance.now();
  checksum += callback().length;
  return {
    operation,
    engine,
    durationMs: performance.now() - start
  };
}

function summarize(samples: BenchSample[]): BenchSummary[] {
  const grouped = new Map<string, BenchSample[]>();
  for (const sample of samples) {
    const key = `${sample.operation}:${sample.engine}`;
    grouped.set(key, [...(grouped.get(key) ?? []), sample]);
  }
  const summaries = Array.from(grouped.values()).map((items) => {
    const sorted = items.map((item) => item.durationMs).sort((left, right) => left - right);
    const total = sorted.reduce((sum, value) => sum + value, 0);
    return {
      operation: items[0].operation,
      engine: items[0].engine,
      runs: items.length,
      meanMs: total / sorted.length,
      p50Ms: percentile(sorted, 0.5),
      p95Ms: percentile(sorted, 0.95),
      minMs: sorted[0],
      maxMs: sorted[sorted.length - 1]
    };
  });
  const jsonRoundtripByOperation = new Map(
    summaries
      .filter((summary) => summary.engine === "json_roundtrip")
      .map((summary) => [summary.operation, summary.meanMs])
  );
  return summaries.map((summary) => ({
    ...summary,
    speedupVsJsonRoundtrip:
      summary.engine === "stateful"
        ? (jsonRoundtripByOperation.get(summary.operation) ?? summary.meanMs) / summary.meanMs
        : undefined
  }));
}

function percentile(sorted: number[], quantile: number): number {
  const index = Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * quantile));
  return sorted[index];
}

function printSummaries(summaries: BenchSummary[], storeJsonBytes: number): void {
  const totalQueries = queryCases().length * queryRuns;
  console.log("# micropuffer stateful wasm benchmark");
  console.log(`namespace: ${namespaceName}`);
  console.log(`rows: ${rowCount}`);
  console.log(`dimensions: ${dimensions}`);
  console.log(`query_runs: ${queryRuns}`);
  console.log(`store_json_bytes_avoided_per_stateful_query: ${storeJsonBytes}`);
  console.log(`store_json_bytes_avoided_total: ${storeJsonBytes * totalQueries}`);
  console.log("");
  console.log("| operation | engine | runs | mean ms | p50 ms | p95 ms | min ms | max ms | x json roundtrip |");
  console.log("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
  for (const summary of summaries) {
    console.log(
      [
        `| ${summary.operation}`,
        summary.engine,
        summary.runs,
        format(summary.meanMs),
        format(summary.p50Ms),
        format(summary.p95Ms),
        format(summary.minMs),
        format(summary.maxMs),
        summary.speedupVsJsonRoundtrip === undefined ? "" : format(summary.speedupVsJsonRoundtrip)
      ].join(" | ") + " |"
    );
  }
  console.log("");
  console.log(
    JSON.stringify(
      {
        rowCount,
        dimensions,
        queryRuns,
        checksum,
        storeJsonBytes,
        totalStoreJsonBytesAvoided: storeJsonBytes * totalQueries,
        summaries
      },
      null,
      2
    )
  );
}

function format(value: number): string {
  return value.toFixed(2);
}

function json(value: JsonValue): string {
  return JSON.stringify(value);
}
