import { performance } from "node:perf_hooks";
import {
  type JsonArray,
  type JsonObject,
  HttpError,
  envOrDefault,
  integerEnv,
  loadEnv,
  parseJsonObject,
  parseMutationResult,
  requiredEnv
} from "./test-utils.ts";
import {
  micropuffer_query,
  micropuffer_write
} from "micropuffer";

type BenchSample = {
  operation: string;
  engine: "live" | "micropuffer";
  durationMs: number;
  rows?: number;
};

type BenchSummary = {
  operation: string;
  engine: "live" | "micropuffer";
  runs: number;
  rows?: number;
  meanMs: number;
  p50Ms: number;
  p95Ms: number;
  minMs: number;
  maxMs: number;
  relativeToLive?: number;
};

type QueryCase = {
  operation: string;
  request: JsonObject;
};

loadEnv();

const apiKey = requiredEnv("TURBOPUFFER_API_KEY");
const region = envOrDefault("TURBOPUFFER_REGION", "gcp-us-central1");
const baseUrl = `https://${region}.turbopuffer.com`;
const rowCount = integerEnv("MICROPUFFER_BENCH_ROWS", 100_000);
const dimensions = integerEnv("MICROPUFFER_BENCH_DIMS", 32);
const batchSize = integerEnv("MICROPUFFER_BENCH_BATCH_SIZE", 1_000);
const queryRuns = integerEnv("MICROPUFFER_BENCH_QUERY_RUNS", 10);
const namespaceName = envOrDefault(
  "MICROPUFFER_BENCH_NAMESPACE",
  `micropuffer-bench-${Date.now()}-${process.pid}`
);
const keepNamespace = envOrDefault("MICROPUFFER_BENCH_KEEP_NAMESPACE", "0") === "1";

let micropufferStore: JsonObject = { namespaces: [] };

await main();

async function main(): Promise<void> {
  validateConfig();
  const rows = Array.from({ length: rowCount }, (_, index) => row(index + 1));
  const samples: BenchSample[] = [];

  await deleteLiveNamespace(namespaceName);

  try {
    samples.push(await time("write", "live", rowCount, () => writeLiveRows(rows)));
    samples.push(timeSync("write", "micropuffer", rowCount, () => writeMicropufferRows(rows)));

    for (const queryCase of queryCases()) {
      await liveQuery(queryCase.request);
      micropufferQuery(queryCase.request);
      for (let run = 0; run < queryRuns; run += 1) {
        samples.push(
          await time(queryCase.operation, "live", undefined, () => liveQuery(queryCase.request))
        );
        samples.push(
          timeSync(queryCase.operation, "micropuffer", undefined, () =>
            micropufferQuery(queryCase.request)
          )
        );
      }
    }

    printSummaries(summarize(samples));
  } finally {
    if (!keepNamespace) {
      await deleteLiveNamespace(namespaceName);
    }
  }
}

function validateConfig(): void {
  if (rowCount <= 0 || dimensions <= 0 || batchSize <= 0 || queryRuns <= 0) {
    throw new Error("benchmark row count, dimensions, batch size, and query runs must be positive.");
  }
}

async function writeLiveRows(rows: JsonObject[]): Promise<void> {
  for (let start = 0; start < rows.length; start += batchSize) {
    const upsertRows = rows.slice(start, start + batchSize);
    await liveWrite(writeRequest(upsertRows, start === 0));
  }
}

function writeMicropufferRows(rows: JsonObject[]): void {
  const result = parseMutationResult(
    micropuffer_write(
      JSON.stringify(micropufferStore),
      namespaceName,
      JSON.stringify(writeRequest(rows, true))
    ),
    "micropuffer write response"
  );
  micropufferStore = result.store;
}

function writeRequest(upsertRows: JsonObject[], includeSchema: boolean): JsonObject {
  const request: JsonObject = {
    distance_metric: "cosine_distance",
    upsert_rows: upsertRows
  };
  if (includeSchema) {
    request.schema = {
      text: { type: "string", full_text_search: true },
      tags: "[]string",
      sparse_vector: {
        type: "{}f16",
        sparse_knn: { distance_metric: "dot_product" }
      }
    };
  }
  return request;
}

function row(id: number): JsonObject {
  const category = `category_${id % 10}`;
  const textBuckets = [
    "walrus arctic mammal",
    "reef coral fish",
    "falcon sky bird",
    "forest fox mammal"
  ];
  const textBucket = textBuckets[id % textBuckets.length];
  return {
    id,
    vector: vectorFor(id),
    sparse_vector: sparseVectorFor(id),
    category,
    public: id % 2,
    score: id % 1_000,
    text: `${textBucket} document ${id}`,
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
      operation: "sparse_top10",
      request: {
        rank_by: ["sparse_vector", "SparseKNN", { "7": 0.7, "12": 0.2 }],
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
    },
    {
      operation: "aggregate_count",
      request: {
        aggregate_by: { count: ["Count"] },
        filters: ["public", "Eq", 1]
      }
    },
    {
      operation: "group_count_top10",
      request: {
        aggregate_by: { count: ["Count"] },
        group_by: ["category"],
        top_k: 10
      }
    }
  ];
}

async function liveWrite(request: JsonObject): Promise<JsonObject> {
  return liveJson("POST", `/v2/namespaces/${encodeURIComponent(namespaceName)}`, request);
}

async function liveQuery(request: JsonObject): Promise<JsonObject> {
  return liveJson("POST", `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`, request);
}

function micropufferQuery(request: JsonObject): JsonObject {
  return parseJsonObject(
    micropuffer_query(JSON.stringify(micropufferStore), namespaceName, JSON.stringify(request)),
    "micropuffer query response"
  );
}

async function liveJson(
  method: "DELETE" | "GET" | "POST",
  path: string,
  body?: JsonObject
): Promise<JsonObject> {
  const init: RequestInit = {
    method,
    headers: {
      Authorization: `Bearer ${apiKey}`,
      "Content-Type": "application/json"
    }
  };
  if (body !== undefined) {
    init.body = JSON.stringify(body);
  }
  const response = await fetch(`${baseUrl}${path}`, init);
  const responseBody = await response.text();
  const parsed = responseBody.length === 0 ? {} : parseJsonObject(responseBody, "live response");
  if (!response.ok) {
    throw new HttpError(response.status, responseBody);
  }
  return parsed;
}

async function deleteLiveNamespace(namespace: string): Promise<void> {
  try {
    await liveJson("DELETE", `/v2/namespaces/${encodeURIComponent(namespace)}`);
  } catch (error) {
    if (error instanceof HttpError && error.status === 404) {
      return;
    }
    throw error;
  }
}

async function time(
  operation: string,
  engine: BenchSample["engine"],
  rows: number | undefined,
  callback: () => Promise<unknown>
): Promise<BenchSample> {
  const start = performance.now();
  await callback();
  return {
    operation,
    engine,
    rows,
    durationMs: performance.now() - start
  };
}

function timeSync(
  operation: string,
  engine: BenchSample["engine"],
  rows: number | undefined,
  callback: () => unknown
): BenchSample {
  const start = performance.now();
  callback();
  return {
    operation,
    engine,
    rows,
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
      rows: items[0].rows,
      meanMs: total / sorted.length,
      p50Ms: percentile(sorted, 0.5),
      p95Ms: percentile(sorted, 0.95),
      minMs: sorted[0],
      maxMs: sorted[sorted.length - 1]
    };
  });
  const liveByOperation = new Map(
    summaries
      .filter((summary) => summary.engine === "live")
      .map((summary) => [summary.operation, summary.meanMs])
  );
  return summaries.map((summary) => ({
    ...summary,
    relativeToLive:
      summary.engine === "micropuffer"
        ? summary.meanMs / (liveByOperation.get(summary.operation) ?? summary.meanMs)
        : undefined
  }));
}

function percentile(sorted: number[], quantile: number): number {
  const index = Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * quantile));
  return sorted[index];
}

function printSummaries(summaries: BenchSummary[]): void {
  console.log(`# micropuffer benchmark`);
  console.log(`namespace: ${namespaceName}`);
  console.log(`rows: ${rowCount}`);
  console.log(`dimensions: ${dimensions}`);
  console.log(`batch_size: ${batchSize}`);
  console.log(`query_runs: ${queryRuns}`);
  console.log("");
  console.log("| operation | engine | runs | rows | mean ms | p50 ms | p95 ms | min ms | max ms | x live |");
  console.log("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
  for (const summary of summaries) {
    console.log(
      [
        `| ${summary.operation}`,
        summary.engine,
        summary.runs,
        summary.rows ?? "",
        format(summary.meanMs),
        format(summary.p50Ms),
        format(summary.p95Ms),
        format(summary.minMs),
        format(summary.maxMs),
        summary.relativeToLive === undefined ? "" : format(summary.relativeToLive)
      ].join(" | ") + " |"
    );
  }
  console.log("");
  console.log(JSON.stringify({ rowCount, dimensions, batchSize, queryRuns, summaries }, null, 2));
}

function format(value: number): string {
  return value.toFixed(2);
}
