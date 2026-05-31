import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import test, { after } from "node:test";
import type { TestContext } from "node:test";
import {
  micropuffer_explain_query,
  micropuffer_list_namespaces,
  micropuffer_metadata,
  micropuffer_query,
  micropuffer_recall,
  micropuffer_schema,
  micropuffer_update_schema,
  micropuffer_warm_cache,
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

type ErrorResult = {
  status: number;
  body: JsonObject;
};

const ENV_PATH = ".env";
const NAMESPACE_PREFIX = "micropuffer-live-parity";
const namespaceName = `${NAMESPACE_PREFIX}-${Date.now()}-${process.pid}`;
const copyNamespaceName = `${namespaceName}-copy`;
const branchNamespaceName = `${namespaceName}-branch`;
const namespacesToDelete = [namespaceName, copyNamespaceName, branchNamespaceName];

loadDotEnv();

const apiKey = requiredEnv("TURBOPUFFER_API_KEY");
const region = envOrDefault("TURBOPUFFER_REGION", "gcp-us-central1");
const baseUrl = `https://${region}.turbopuffer.com`;

let micropufferStore: JsonObject = { namespaces: [] };

after(async () => {
  for (const namespace of namespacesToDelete) {
    await deleteLiveNamespace(namespace);
  }
});

test("micropuffer wasm matches live turbopuffer for core query and workspace operations", async (context) => {
  await deleteLiveNamespace(namespaceName);
  await deleteLiveNamespace(copyNamespaceName);
  await deleteLiveNamespace(branchNamespaceName);

  const seedWrite: JsonObject = {
    distance_metric: "cosine_distance",
    schema: {
      title: { type: "string", fuzzy: true },
      text: { type: "string", full_text_search: true },
      published_at: "datetime",
      tags: "[]string",
      sparse_vector: {
        type: "{}f16",
        sparse_knn: { distance_metric: "dot_product" }
      }
    },
    upsert_rows: [
      {
        id: 1,
        vector: [1.0, 0.0],
        sparse_vector: { "0": 1.0, "2": 0.5 },
        category: "mammal",
        public: 1,
        title: "walrus den",
        text: "walrus narwhal arctic mammal",
        score: 10,
        tags: ["arctic", "mammal"],
        published_at: "2026-05-30T00:00:00Z"
      },
      {
        id: 2,
        vector: [0.0, 1.0],
        sparse_vector: { "1": 1.0 },
        category: "fish",
        public: 0,
        title: "reef fish",
        text: "pufferfish clownfish swordfish",
        score: 7,
        tags: ["fish"],
        published_at: "2026-05-29T00:00:00Z"
      },
      {
        id: 3,
        vector: [0.8, 0.2],
        sparse_vector: { "0": 0.8, "2": 0.2 },
        category: "mammal",
        public: 1,
        title: "quick walrus",
        text: "quick walrus sea mammal",
        score: 5,
        tags: ["mammal", "sea"],
        published_at: "2026-05-31T00:00:00Z"
      }
    ]
  };

  assertParity(await liveWrite(namespaceName, seedWrite), miniWrite(namespaceName, seedWrite));

  const vectorQuery: JsonObject = {
    rank_by: ["vector", "ANN", [1.0, 0.0]],
    limit: 10,
    filters: [
      "And",
      [
        ["category", "Eq", "mammal"],
        ["public", "Eq", 1]
      ]
    ],
    include_attributes: ["category", "score"]
  };
  assertParity(await liveQuery(namespaceName, vectorQuery), miniQuery(namespaceName, vectorQuery));

  const bm25Query: JsonObject = {
    rank_by: ["text", "BM25", "quick walrus"],
    limit: 10,
    filters: ["category", "Eq", "mammal"],
    include_attributes: ["text"]
  };
  assertParity(await liveQuery(namespaceName, bm25Query), miniQuery(namespaceName, bm25Query));

  const sparseQuery: JsonObject = {
    rank_by: ["sparse_vector", "SparseKNN", { "0": 1.0, "2": 0.1 }],
    limit: 10,
    include_attributes: ["category"]
  };
  assertParity(await liveQuery(namespaceName, sparseQuery), miniQuery(namespaceName, sparseQuery));

  const excludeAttributesQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 2,
    exclude_attributes: ["vector", "sparse_vector", "text"]
  };
  assertParity(
    await liveQuery(namespaceName, excludeAttributesQuery),
    miniQuery(namespaceName, excludeAttributesQuery)
  );

  const base64VectorQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    include_attributes: ["vector"],
    vector_encoding: "base64"
  };
  assertParity(await liveQuery(namespaceName, base64VectorQuery), miniQuery(namespaceName, base64VectorQuery));

  const fuzzyQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    filters: [
      "title",
      "Fuzzy",
      "walruss",
      { max_edit_distance: [{ min_query_chars: 6, distance: 1 }] }
    ],
    include_attributes: ["title"]
  };
  assertParity(await liveQuery(namespaceName, fuzzyQuery), miniQuery(namespaceName, fuzzyQuery));

  await assertColumnAndConditionParity();

  const aggregateQuery: JsonObject = {
    aggregate_by: { count: ["Count"] },
    group_by: [{ tag: ["ForEachUnique", "tags"] }],
    top_k: 10
  };
  assertParity(await liveQuery(namespaceName, aggregateQuery), miniQuery(namespaceName, aggregateQuery));

  const multiQuery: JsonObject = {
    queries: [
      { rank_by: ["vector", "ANN", [1.0, 0.0]], limit: 1 },
      { rank_by: ["text", "BM25", "walrus"], limit: 2 }
    ]
  };
  assertParity(await liveQuery(namespaceName, multiQuery), miniQuery(namespaceName, multiQuery));

  const patchByFilter: JsonObject = {
    patch_by_filter: {
      filters: ["category", "Eq", "fish"],
      patch: { category: "reef", score: 8 }
    },
    return_affected_ids: true
  };
  assertParity(await liveWrite(namespaceName, patchByFilter), miniWrite(namespaceName, patchByFilter));

  const patchedLookup: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    include_attributes: ["category", "score"]
  };
  assertParity(await liveQuery(namespaceName, patchedLookup), miniQuery(namespaceName, patchedLookup));

  const deleteByFilter: JsonObject = {
    delete_by_filter: ["score", "Lt", 6],
    return_affected_ids: true
  };
  assertParity(await liveWrite(namespaceName, deleteByFilter), miniWrite(namespaceName, deleteByFilter));

  const afterDeleteLookup: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    include_attributes: ["category", "score"]
  };
  assertParity(await liveQuery(namespaceName, afterDeleteLookup), miniQuery(namespaceName, afterDeleteLookup));

  await assertCopyParity(afterDeleteLookup);
  await assertListNamespaceParity();
  await assertMetadataParity();
  await assertSchemaParity();
  await assertSchemaUpdateParity();
  await assertWarmCacheParity();
  await assertRecallParity();
  await assertExplainQueryParity(context);
  await assertErrorParity();
  await assertBranchParityIfAllowed(context, afterDeleteLookup);
});

async function assertColumnAndConditionParity(): Promise<void> {
  const columnWrite: JsonObject = {
    upsert_columns: {
      id: [4, 5],
      vector: [
        [0.5, 0.5],
        [0.4, 0.6]
      ],
      sparse_vector: [{ "0": 0.4 }, { "1": 0.6 }],
      title: ["column walrus", "blocked walrus"],
      category: ["mammal", "mammal"],
      public: [1, 1],
      text: ["column walrus mammal", "blocked walrus mammal"],
      score: [2, 20],
      tags: [["mammal"], ["mammal"]],
      published_at: ["2026-05-28T00:00:00Z", "2026-05-27T00:00:00Z"]
    }
  };
  assertParity(await liveWrite(namespaceName, columnWrite), miniWrite(namespaceName, columnWrite));

  const conditionalWrite: JsonObject = {
    upsert_rows: [
      {
        id: 4,
        vector: [0.5, 0.5],
        sparse_vector: { "0": 0.5 },
        title: "condition pass",
        category: "mammal",
        public: 1,
        text: "condition pass mammal",
        score: 3,
        tags: ["mammal"],
        published_at: "2026-05-28T00:00:00Z"
      },
      {
        id: 5,
        vector: [0.4, 0.6],
        sparse_vector: { "1": 0.6 },
        title: "condition blocked",
        category: "mammal",
        public: 1,
        text: "condition blocked mammal",
        score: 10,
        tags: ["mammal"],
        published_at: "2026-05-27T00:00:00Z"
      },
      {
        id: 6,
        vector: [0.3, 0.7],
        sparse_vector: { "1": 0.7 },
        title: "condition insert",
        category: "mammal",
        public: 1,
        text: "condition insert mammal",
        score: 1,
        tags: ["mammal"],
        published_at: "2026-05-26T00:00:00Z"
      }
    ],
    upsert_condition: ["score", "Lt", { "$ref_new": "score" }],
    return_affected_ids: true
  };
  assertParity(
    await liveWrite(namespaceName, conditionalWrite),
    miniWrite(namespaceName, conditionalWrite)
  );

  const conditionalPatch: JsonObject = {
    patch_columns: {
      id: [4, 6],
      title: ["patched column", "patched insert"],
      score: [4, 100]
    },
    patch_condition: ["public", "Eq", 1],
    return_affected_ids: true
  };
  assertParity(
    await liveWrite(namespaceName, conditionalPatch),
    miniWrite(namespaceName, conditionalPatch)
  );

  const conditionalDelete: JsonObject = {
    deletes: [5],
    delete_condition: ["score", "Gte", 20],
    return_affected_ids: true
  };
  assertParity(
    await liveWrite(namespaceName, conditionalDelete),
    miniWrite(namespaceName, conditionalDelete)
  );

  const lookup: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    include_attributes: ["title", "score", "category"]
  };
  assertParity(await liveQuery(namespaceName, lookup), miniQuery(namespaceName, lookup));
}

async function assertCopyParity(lookup: JsonObject): Promise<void> {
  const liveCopy = await liveWrite(copyNamespaceName, { copy_from_namespace: namespaceName });
  const miniCopy = miniWrite(copyNamespaceName, { copy_from_namespace: namespaceName });
  assert.equal(liveCopy.rows_affected, miniCopy.rows_affected);
  assertParity(await liveQuery(copyNamespaceName, lookup), miniQuery(copyNamespaceName, lookup));
}

async function assertListNamespaceParity(): Promise<void> {
  const live = await liveJson(
    "GET",
    `/v1/namespaces?prefix=${encodeURIComponent(namespaceName)}&page_size=10`
  );
  const mini = parseJsonObject(
    micropuffer_list_namespaces(
      JSON.stringify(micropufferStore),
      JSON.stringify({ prefix: namespaceName, page_size: 10 })
    ),
    "micropuffer list response"
  );
  assert.deepEqual(namespaceIds(live), namespaceIds(mini));
}

async function assertMetadataParity(): Promise<void> {
  const live = await liveJson("GET", `/v1/namespaces/${encodeURIComponent(namespaceName)}/metadata`);
  const mini = parseJsonObject(
    micropuffer_metadata(JSON.stringify(micropufferStore), namespaceName),
    "micropuffer metadata response"
  );
  assert.deepEqual(schemaTypes(live), schemaTypes(mini));
}

async function assertSchemaParity(): Promise<void> {
  const live = await liveJson("GET", `/v1/namespaces/${encodeURIComponent(namespaceName)}/schema`);
  const mini = parseJsonObject(
    micropuffer_schema(JSON.stringify(micropufferStore), namespaceName),
    "micropuffer schema response"
  );
  assert.deepEqual(schemaTypes({ schema: live }), schemaTypes({ schema: mini }));
}

async function assertSchemaUpdateParity(): Promise<void> {
  const schemaUpdate: JsonObject = {
    category: {
      type: "string",
      regex: true,
      filterable: true
    }
  };
  const live = await liveJson(
    "POST",
    `/v1/namespaces/${encodeURIComponent(namespaceName)}/schema`,
    schemaUpdate
  );
  const miniResult = parseMutationResult(
    micropuffer_update_schema(
      JSON.stringify(micropufferStore),
      namespaceName,
      JSON.stringify(schemaUpdate)
    ),
    "micropuffer schema update response"
  );
  micropufferStore = miniResult.store;
  assert.deepEqual(schemaTypes({ schema: live }), schemaTypes({ schema: miniResult.response }));
}

async function assertWarmCacheParity(): Promise<void> {
  const live = await liveJson(
    "GET",
    `/v1/namespaces/${encodeURIComponent(namespaceName)}/hint_cache_warm`
  );
  const mini = parseJsonObject(
    micropuffer_warm_cache(JSON.stringify(micropufferStore), namespaceName),
    "micropuffer warm cache response"
  );
  assert.equal(live.status, mini.status);
}

async function assertRecallParity(): Promise<void> {
  const request: JsonObject = {
    num: 1,
    top_k: 2,
    filters: ["public", "Eq", 1]
  };
  const live = await liveJson(
    "POST",
    `/v1/namespaces/${encodeURIComponent(namespaceName)}/_debug/recall`,
    request
  );
  const mini = parseJsonObject(
    micropuffer_recall(JSON.stringify(micropufferStore), namespaceName, JSON.stringify(request)),
    "micropuffer recall response"
  );
  assert.equal(typeof live.avg_recall, "number");
  assert.equal(typeof live.avg_exhaustive_count, "number");
  assert.equal(typeof live.avg_ann_count, "number");
  assert.equal(mini.avg_recall, 1);
  assert.equal(typeof mini.avg_exhaustive_count, "number");
  assert.equal(typeof mini.avg_ann_count, "number");
}

async function assertExplainQueryParity(context: TestContext): Promise<void> {
  const request: JsonObject = {
    rank_by: ["text", "BM25", "walrus"],
    filters: ["public", "Eq", 1],
    limit: 3
  };
  const mini = parseJsonObject(
    micropuffer_explain_query(
      JSON.stringify(micropufferStore),
      namespaceName,
      JSON.stringify(request)
    ),
    "micropuffer explain query response"
  );
  assert.equal(typeof mini.plan_text, "string");
  try {
    const live = await liveJson(
      "POST",
      `/v2/namespaces/${encodeURIComponent(namespaceName)}/explain_query`,
      request
    );
    assert.deepEqual(Object.keys(live).sort(), Object.keys(mini).sort());
  } catch (error) {
    if (
      error instanceof HttpError &&
      (error.status === 400 || error.status === 403 || error.status === 404)
    ) {
      context.diagnostic(
        `Skipping explain_query live shape parity: live endpoint returned HTTP ${error.status}.`
      );
      return;
    }
    throw error;
  }
}

async function assertErrorParity(): Promise<void> {
  const invalidQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    include_attributes: ["title"],
    exclude_attributes: ["text"]
  };
  const live = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidQuery
  );
  const mini = miniQueryError(namespaceName, invalidQuery);
  assert.equal(live.status, mini.status);
  assert.equal(live.body.status, mini.body.status);
  assert.equal(live.body.error, mini.body.error);
}

async function assertBranchParityIfAllowed(
  context: TestContext,
  lookup: JsonObject
): Promise<void> {
  try {
    await liveWrite(branchNamespaceName, { branch_from_namespace: namespaceName });
  } catch (error) {
    if (error instanceof HttpError && error.status === 403) {
      context.diagnostic("Skipping branch parity: API key is not permitted to branch namespaces.");
      return;
    }
    throw error;
  }
  miniWrite(branchNamespaceName, { branch_from_namespace: namespaceName });
  assertParity(await liveQuery(branchNamespaceName, lookup), miniQuery(branchNamespaceName, lookup));
}

function miniWrite(namespace: string, request: JsonObject): JsonObject {
  const result = parseMutationResult(
    micropuffer_write(JSON.stringify(micropufferStore), namespace, JSON.stringify(request)),
    "micropuffer write response"
  );
  micropufferStore = result.store;
  return result.response;
}

function miniQuery(namespace: string, request: JsonObject): JsonObject {
  return parseJsonObject(
    micropuffer_query(JSON.stringify(micropufferStore), namespace, JSON.stringify(request)),
    "micropuffer query response"
  );
}

async function liveWrite(namespace: string, request: JsonObject): Promise<JsonObject> {
  return liveJson("POST", `/v2/namespaces/${encodeURIComponent(namespace)}`, request);
}

async function liveQuery(namespace: string, request: JsonObject): Promise<JsonObject> {
  return liveJson("POST", `/v2/namespaces/${encodeURIComponent(namespace)}/query`, request);
}

async function liveError(
  method: "DELETE" | "GET" | "POST",
  path: string,
  body?: JsonObject
): Promise<ErrorResult> {
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
  if (response.ok) {
    throw new Error(`Expected live request to fail, got HTTP ${response.status}.`);
  }
  return {
    status: response.status,
    body: parseJsonObject(responseBody, "live error response")
  };
}

function miniQueryError(namespace: string, request: JsonObject): ErrorResult {
  try {
    miniQuery(namespace, request);
  } catch (error) {
    return {
      status: 400,
      body: {
        status: "error",
        error: errorText(error)
      }
    };
  }
  throw new Error("Expected micropuffer query to fail.");
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

function assertParity(live: JsonObject, mini: JsonObject): void {
  assert.deepEqual(stableJson(live), stableJson(mini));
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
      if (key === "rows_remaining" && item === false) {
        continue;
      }
      stable[key] = stableJson(item);
    }
    return stable;
  }
  if (typeof value === "number") {
    return Math.round(value * 1_000) / 1_000;
  }
  return value;
}

function namespaceIds(response: JsonObject): string[] {
  const namespaces = response.namespaces;
  if (!Array.isArray(namespaces)) {
    throw new Error("namespace list response did not contain namespaces.");
  }
  return namespaces.map(namespaceId).sort();
}

function namespaceId(value: JsonValue): string {
  if (!isJsonObject(value) || typeof value.id !== "string") {
    throw new Error("namespace list item did not contain an id.");
  }
  return value.id;
}

function schemaTypes(metadata: JsonObject): JsonObject {
  const schema = metadata.schema;
  if (!isJsonObject(schema)) {
    throw new Error("metadata did not contain a schema object.");
  }
  const types: JsonObject = {};
  for (const [attribute, definition] of Object.entries(schema)) {
    if (typeof definition === "string") {
      types[attribute] = definition;
    } else if (isJsonObject(definition) && typeof definition.type === "string") {
      types[attribute] = definition.type;
    }
  }
  return types;
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

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
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

class HttpError extends Error {
  readonly status: number;
  readonly body: string;

  constructor(status: number, body: string) {
    super(`turbopuffer request failed with HTTP ${status}: ${body}`);
    this.status = status;
    this.body = body;
  }
}
