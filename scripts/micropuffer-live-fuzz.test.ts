import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import test, { after } from "node:test";
import {
  micropuffer_query,
  micropuffer_write
} from "../pkg/micropuffer.js";

type JsonPrimitive = boolean | null | number | string;
type JsonArray = JsonValue[];
type JsonObject = { [key: string]: JsonValue };
type JsonValue = JsonArray | JsonObject | JsonPrimitive;

type MutationResult = {
  response: JsonObject;
  store: JsonObject;
};

const ENV_PATH = ".env";
const namespaceName = `micropuffer-live-fuzz-${Date.now()}-${process.pid}`;
const seed = 0x5eed_2026;

loadDotEnv();

const apiKey = requiredEnv("TURBOPUFFER_API_KEY");
const region = envOrDefault("TURBOPUFFER_REGION", "gcp-us-central1");
const baseUrl = `https://${region}.turbopuffer.com`;

let micropufferStore: JsonObject = { namespaces: [] };

after(async () => {
  await deleteLiveNamespace(namespaceName);
});

test("micropuffer wasm fuzzes live turbopuffer query parity", async () => {
  await deleteLiveNamespace(namespaceName);
  const seedWrite = buildSeedWrite();
  await liveWrite(seedWrite);
  miniWrite(seedWrite);

  const cases = buildFuzzCases();
  for (const fuzzCase of cases) {
    const live = await liveQuery(fuzzCase.request);
    const mini = miniQuery(fuzzCase.request);
    assert.deepEqual(
      stableJson(live),
      stableJson(mini),
      `fuzz case ${fuzzCase.name}: ${JSON.stringify(fuzzCase.request)}`
    );
  }
});

function buildSeedWrite(): JsonObject {
  return {
    distance_metric: "cosine_distance",
    schema: {
      text: { type: "string", full_text_search: true },
      name: { type: "string", regex: true },
      published_at: "datetime",
      tags: "[]string",
      scores: "[]int",
      sparse_vector: {
        type: "{}f16",
        sparse_knn: { distance_metric: "dot_product" }
      }
    },
    upsert_rows: [
      row(1, [1, 0], { "0": 1, "2": 0.5 }, "mammal", 1, "alpha", "walrus narwhal arctic mammal", 10, ["arctic", "mammal"], [1, 5], "2026-05-30T00:00:00Z"),
      row(2, [0, 1], { "1": 1 }, "fish", 0, "bravo", "pufferfish clownfish swordfish", 7, ["fish"], [3, 7], "2026-05-29T00:00:00Z"),
      row(3, [0.8, 0.2], { "0": 0.8, "2": 0.2 }, "mammal", 1, "charlie", "quick walrus sea mammal", 5, ["mammal", "sea"], [2, 9], "2026-05-31T00:00:00Z"),
      row(4, [0.25, 0.75], { "1": 0.9, "3": 0.4 }, "bird", 1, "delta", "swift falcon sky hunter", 12, ["bird", "sky"], [4, 8], "2026-05-28T00:00:00Z"),
      row(5, [0.6, 0.4], { "0": 0.3, "2": 0.9 }, "mammal", 0, "echo", "brown fox quick den", 3, ["mammal", "forest"], [0, 3], "2026-05-27T00:00:00Z"),
      row(6, [0.1, 0.9], { "1": 0.7, "3": 0.1 }, "fish", 1, "foxtrot", "reef fish coral clownfish", 9, ["fish", "reef"], [6, 9], "2026-05-26T00:00:00Z")
    ]
  };
}

function row(
  id: number,
  vector: JsonArray,
  sparseVector: JsonObject,
  category: string,
  isPublic: number,
  name: string,
  text: string,
  score: number,
  tags: JsonArray,
  scores: JsonArray,
  publishedAt: string
): JsonObject {
  return {
    id,
    vector,
    sparse_vector: sparseVector,
    category,
    public: isPublic,
    name,
    text,
    score,
    tags,
    scores,
    published_at: publishedAt
  };
}

function buildFuzzCases(): { name: string; request: JsonObject }[] {
  const rng = new Rng(seed);
  const rankers: JsonValue[] = [
    ["id", "asc"],
    ["score", "desc"],
    ["category", "asc"],
    [["category", "asc"], ["score", "desc"]],
    ["vector", "ANN", [0.9, 0.1]],
    ["vector", "kNN", [0.9, 0.1]],
    ["text", "BM25", "quick walrus"],
    ["text", "BM25", "clown", { last_as_prefix: true }],
    ["sparse_vector", "SparseKNN", { "0": 1.0, "2": 0.1 }],
    ["Sum", [["text", "BM25", "quick"], ["Product", 2, ["category", "Eq", "mammal"]]]],
    ["Max", [["text", "BM25", "fish"], ["Product", 1.5, ["public", "Eq", 1]]]]
  ];
  const filters: (JsonValue | undefined)[] = [
    undefined,
    ["category", "Eq", "mammal"],
    ["category", "NotEq", "fish"],
    ["category", "In", ["mammal", "bird"]],
    ["category", "NotIn", ["fish"]],
    ["tags", "Contains", "mammal"],
    ["tags", "NotContains", "reef"],
    ["tags", "ContainsAny", ["reef", "sky"]],
    ["tags", "NotContainsAny", ["desert"]],
    ["score", "Lt", 10],
    ["score", "Lte", 9],
    ["score", "Gt", 5],
    ["score", "Gte", 7],
    ["scores", "AnyLt", 2],
    ["scores", "AnyLte", 3],
    ["scores", "AnyGt", 8],
    ["scores", "AnyGte", 9],
    ["name", "Glob", "c*"],
    ["name", "IGlob", "E*"],
    ["name", "Regex", "^(alpha|delta)$"],
    ["text", "ContainsAllTokens", "quick mammal"],
    ["text", "ContainsAnyToken", ["coral", "narwhal"]],
    ["text", "ContainsTokenSequence", "sea mammal"],
    ["And", [["category", "Eq", "mammal"], ["public", "Eq", 1]]],
    ["Or", [["category", "Eq", "bird"], ["tags", "Contains", "reef"]]],
    ["Not", ["category", "Eq", "fish"]]
  ];
  const includes: JsonValue[] = [
    ["category"],
    ["category", "score"],
    ["name", "tags"],
    true
  ];
  const cases: { name: string; request: JsonObject }[] = [];
  for (let index = 0; index < 80; index += 1) {
    const rankBy = pick(rng, rankers);
    const filter = pick(rng, filters);
    const request: JsonObject = {
      rank_by: rankBy,
      limit: 1 + rng.nextInt(5),
      include_attributes: pick(rng, includes)
    };
    if (filter !== undefined) {
      request.filters = filter;
    }
    if (isKnn(rankBy) && filter === undefined) {
      request.filters = ["public", "Eq", 1];
    }
    cases.push({ name: `rank-${index}`, request });
  }
  cases.push({
    name: "aggregate-count-public",
    request: {
      aggregate_by: { count: ["Count"] },
      filters: ["public", "Eq", 1]
    }
  });
  cases.push({
    name: "aggregate-group-category",
    request: {
      aggregate_by: { count: ["Count"] },
      group_by: ["category"],
      top_k: 10
    }
  });
  cases.push({
    name: "multi-query",
    request: {
      queries: [
        { rank_by: ["vector", "ANN", [1, 0]], limit: 2 },
        { rank_by: ["text", "BM25", "fish"], limit: 2 }
      ]
    }
  });
  return cases;
}

function isKnn(rankBy: JsonValue): boolean {
  return Array.isArray(rankBy) && rankBy[1] === "kNN";
}

function pick<T>(rng: Rng, values: T[]): T {
  return values[rng.nextInt(values.length)];
}

function miniWrite(request: JsonObject): JsonObject {
  const result = parseMutationResult(
    micropuffer_write(JSON.stringify(micropufferStore), namespaceName, JSON.stringify(request)),
    "micropuffer write response"
  );
  micropufferStore = result.store;
  return result.response;
}

function miniQuery(request: JsonObject): JsonObject {
  return parseJsonObject(
    micropuffer_query(JSON.stringify(micropufferStore), namespaceName, JSON.stringify(request)),
    "micropuffer query response"
  );
}

async function liveWrite(request: JsonObject): Promise<JsonObject> {
  return liveJson("POST", `/v2/namespaces/${encodeURIComponent(namespaceName)}`, request);
}

async function liveQuery(request: JsonObject): Promise<JsonObject> {
  return liveJson("POST", `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`, request);
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

function stableJson(value: JsonValue): JsonValue {
  if (Array.isArray(value)) {
    return value.map(stableJson);
  }
  if (isJsonObject(value)) {
    const stable: JsonObject = {};
    for (const [key, item] of Object.entries(value)) {
      if (key === "billing" || key === "performance" || key === "status" || key === "message") {
        continue;
      }
      stable[key] = stableJson(item);
    }
    return stable;
  }
  if (typeof value === "number") {
    return Math.round(value * 1_000) / 1_000;
  }
  if (typeof value === "string") {
    return value.replace(".000000000Z", "Z");
  }
  return value;
}

function parseMutationResult(raw: string, label: string): MutationResult {
  const parsed = parseJsonObject(raw, label);
  if (!isJsonObject(parsed.response) || !isJsonObject(parsed.store)) {
    throw new Error(`${label} did not contain response and store objects.`);
  }
  return {
    response: parsed.response,
    store: parsed.store
  };
}

function parseJsonObject(raw: string, label: string): JsonObject {
  const parsed: unknown = JSON.parse(raw);
  if (!isJsonObject(parsed)) {
    throw new Error(`${label} was not a JSON object.`);
  }
  return parsed;
}

function isJsonObject(value: unknown): value is JsonObject {
  return isRecord(value) && Object.values(value).every(isJsonValue);
}

function isJsonValue(value: unknown): value is JsonValue {
  if (
    value === null ||
    typeof value === "boolean" ||
    typeof value === "number" ||
    typeof value === "string"
  ) {
    return true;
  }
  if (Array.isArray(value)) {
    return value.every(isJsonValue);
  }
  return isJsonObject(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requiredEnv(name: string): string {
  const value = process.env[name];
  if (value === undefined || value.length === 0) {
    throw new Error(`${name} is required. Put it in ${ENV_PATH}.`);
  }
  return value;
}

function envOrDefault(name: string, fallback: string): string {
  const value = process.env[name];
  return value === undefined || value.length === 0 ? fallback : value;
}

function loadDotEnv(): void {
  let raw = "";
  try {
    raw = readFileSync(ENV_PATH, "utf8");
  } catch {
    return;
  }
  for (const line of raw.split("\n")) {
    const trimmed = line.trim();
    if (trimmed.length === 0 || trimmed.startsWith("#")) {
      continue;
    }
    const separator = trimmed.indexOf("=");
    if (separator <= 0) {
      continue;
    }
    const key = trimmed.slice(0, separator).trim();
    let value = trimmed.slice(separator + 1).trim();
    if (
      ((value.startsWith('"') && value.endsWith('"')) ||
        (value.startsWith("'") && value.endsWith("'"))) &&
      value.length >= 2
    ) {
      value = value.slice(1, -1);
    }
    if (process.env[key] === undefined) {
      process.env[key] = value;
    }
  }
}

class Rng {
  private state: number;

  constructor(state: number) {
    this.state = state >>> 0;
  }

  nextInt(maxExclusive: number): number {
    this.state = (Math.imul(1_664_525, this.state) + 1_013_904_223) >>> 0;
    return this.state % maxExclusive;
  }
}

class HttpError extends Error {
  readonly status: number;
  readonly body: string;

  constructor(status: number, body: string) {
    super(`turbopuffer request failed with HTTP ${status}: ${body}`);
    this.status = status;
    this.body = body;
  }
}
