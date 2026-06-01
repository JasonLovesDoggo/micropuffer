import fc from "fast-check";
import { afterAll, test } from "vitest";
import {
  type JsonArray,
  type JsonObject,
  type JsonValue,
  HttpError,
  envOrDefault,
  expectJsonParity,
  loadEnv,
  parseJsonObject,
  requiredEnv
} from "./test-utils";
import {
  Micropuffer
} from "micropuffer";

const namespaceName = `micropuffer-live-fuzz-${Date.now()}-${process.pid}`;
const seed = 0x5eed_2026;

loadEnv();

const apiKey = requiredEnv("TURBOPUFFER_API_KEY");
const region = envOrDefault("TURBOPUFFER_REGION", "gcp-us-central1");
const baseUrl = `https://${region}.turbopuffer.com`;

const micropuffer = new Micropuffer();

afterAll(async () => {
  await deleteLiveNamespace(namespaceName);
});

test("micropuffer wasm fuzzes live turbopuffer query parity", async () => {
  await deleteLiveNamespace(namespaceName);
  const seedWrite = buildSeedWrite();
  await liveWrite(seedWrite);
  miniWrite(seedWrite);

  await fc.assert(
    fc.asyncProperty(queryCaseArbitrary(), async (fuzzCase) => {
      const live = await liveQuery(fuzzCase.request);
      const mini = miniQuery(fuzzCase.request);
      expectJsonParity(
        live,
        mini,
        `fuzz case ${fuzzCase.name}: ${JSON.stringify(fuzzCase.request)}`
      );
    }),
    { numRuns: 160, seed }
  );

  for (const fuzzCase of fixedCases()) {
    expectJsonParity(await liveQuery(fuzzCase.request), miniQuery(fuzzCase.request), fuzzCase.name);
  }
});

function buildSeedWrite(): JsonObject {
  return {
    distance_metric: "cosine_distance",
    schema: {
      text: { type: "string", full_text_search: true },
      tokens: {
        type: "[]string",
        full_text_search: { tokenizer: "pre_tokenized_array" }
      },
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
      row(1, [1, 0], { "0": 1, "2": 0.5 }, "mammal", 1, "alpha", "walrus narwhal arctic mammal", ["walrus", "narwhal", "mammal"], 10, ["arctic", "mammal"], [1, 5], "2026-05-30T00:00:00Z"),
      row(2, [0, 1], { "1": 1 }, "fish", 0, "bravo", "pufferfish clownfish swordfish", ["pufferfish", "clownfish", "swordfish"], 7, ["fish"], [3, 7], "2026-05-29T00:00:00Z"),
      row(3, [0.8, 0.2], { "0": 0.8, "2": 0.2 }, "mammal", 1, "charlie", "quick walrus sea mammal", ["quick", "walrus", "mammal"], 5, ["mammal", "sea"], [2, 9], "2026-05-31T00:00:00Z"),
      row(4, [0.25, 0.75], { "1": 0.9, "3": 0.4 }, "bird", 1, "delta", "swift falcon sky hunter", ["swift", "falcon", "sky"], 12, ["bird", "sky"], [4, 8], "2026-05-28T00:00:00Z"),
      row(5, [0.6, 0.4], { "0": 0.3, "2": 0.9 }, "mammal", 0, "echo", "brown fox quick den", ["brown", "fox", "quick"], 3, ["mammal", "forest"], [0, 3], "2026-05-27T00:00:00Z"),
      row(6, [0.1, 0.9], { "1": 0.7, "3": 0.1 }, "fish", 1, "foxtrot", "reef fish coral clownfish", ["reef", "fish", "coral"], 9, ["fish", "reef"], [6, 9], "2026-05-26T00:00:00Z")
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
  tokens: JsonArray,
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
    tokens,
    score,
    tags,
    scores,
    published_at: publishedAt
  };
}

type FuzzCase = { name: string; request: JsonObject };

const RANKERS: JsonValue[] = [
  ["id", "asc"],
  ["score", "desc"],
  ["category", "asc"],
  [["category", "asc"], ["score", "desc"]],
  ["vector", "ANN", [0.9, 0.1]],
  ["vector", "kNN", [0.9, 0.1]],
  ["text", "BM25", "quick walrus"],
  ["text", "BM25", "clown", { last_as_prefix: true }],
  ["tokens", "BM25", ["walrus"]],
  ["sparse_vector", "SparseKNN", { "0": 1.0, "2": 0.1 }],
  ["Sum", [["text", "BM25", "quick"], ["Product", 2, ["category", "Eq", "mammal"]]]],
  ["Max", [["text", "BM25", "fish"], ["Product", 1.5, ["public", "Eq", 1]]]]
];

const FILTERS: JsonValue[] = [
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
  ["name", "NotGlob", "z*"],
  ["name", "IGlob", "E*"],
  ["name", "NotIGlob", "z*"],
  ["name", "Regex", "^(alpha|delta)$"],
  ["text", "ContainsAllTokens", "quick mammal"],
  ["text", "ContainsAnyToken", "coral narwhal"],
  ["text", "ContainsTokenSequence", "sea mammal"],
  ["tokens", "ContainsAllTokens", ["walrus", "mammal"]],
  ["tokens", "ContainsAnyToken", ["reef", "narwhal"]],
  ["tokens", "ContainsTokenSequence", ["walrus", "narwhal"]],
  ["And", [["category", "Eq", "mammal"], ["public", "Eq", 1]]],
  ["Or", [["category", "Eq", "bird"], ["tags", "Contains", "reef"]]],
  ["Not", ["category", "Eq", "fish"]]
];

const PROJECTIONS: JsonObject[] = [
  {},
  { include_attributes: [] },
  { include_attributes: ["category"] },
  { include_attributes: ["category", "score"] },
  { include_attributes: ["vector", "category"] },
  { include_attributes: ["vector"], vector_encoding: "base64" },
  { include_attributes: ["name", "tags"] },
  { include_attributes: false },
  { include_attributes: true },
  { exclude_attributes: ["vector", "sparse_vector", "text"] }
];

function queryCaseArbitrary(): fc.Arbitrary<FuzzCase> {
  return fc
    .record({
      caseId: fc.nat(),
      rankBy: fc.constantFrom(...RANKERS),
      filter: fc.option(fc.constantFrom(...FILTERS), { nil: undefined }),
      limit: fc.integer({ min: 1, max: 5 }),
      limitField: fc.constantFrom("limit", "top_k"),
      projection: fc.constantFrom(...PROJECTIONS)
    })
    .map(({ caseId, rankBy, filter, limit, limitField, projection }) => {
      const request: JsonObject = {
        rank_by: rankBy
      };
      if (limitField === "top_k") {
        request.top_k = limit;
      } else {
        request.limit = limit;
      }
      for (const [key, value] of Object.entries(projection)) {
        request[key] = value;
      }
      if (filter !== undefined) {
        request.filters = filter;
      }
      if (isKnn(rankBy) && filter === undefined) {
        request.filters = ["public", "Eq", 1];
      }
      return { name: `rank-${caseId}`, request };
    });
}

function fixedCases(): FuzzCase[] {
  return [
    {
      name: "limit-per-category",
      request: {
        rank_by: ["category", "asc"],
        limit: { total: 5, per: { attributes: ["category"], limit: 1 } },
        include_attributes: ["category", "score"]
      }
    },
    {
      name: "base64-vector-projection",
      request: {
        rank_by: ["vector", "ANN", [1, 0]],
        limit: 3,
        include_attributes: ["vector"],
        vector_encoding: "base64"
      }
    },
    {
      name: "pretokenized-token-sequence",
      request: {
        rank_by: ["id", "asc"],
        filters: ["tokens", "ContainsTokenSequence", ["walrus", "narwhal"]],
        limit: 10,
        include_attributes: ["tokens"]
      }
    },
    {
      name: "aggregate-sum-public",
      request: {
        aggregate_by: { score_sum: ["Sum", "score"] },
        filters: ["public", "Eq", 1]
      }
    },
    {
      name: "aggregate-for-each-tag",
      request: {
        aggregate_by: { count: ["Count"] },
        group_by: [{ tag: ["ForEachUnique", "tags"] }],
        top_k: 10
      }
    },
    {
      name: "aggregate-count-public",
      request: {
        aggregate_by: { count: ["Count"] },
        filters: ["public", "Eq", 1]
      }
    },
    {
      name: "aggregate-count-id-deprecated",
      request: {
        aggregate_by: { count: ["Count", "id"] },
        filters: ["public", "Eq", 1]
      }
    },
    {
      name: "aggregate-group-category",
      request: {
        aggregate_by: { count: ["Count"] },
        group_by: ["category"],
        top_k: 10
      }
    },
    {
      name: "multi-query",
      request: {
        queries: [
          { rank_by: ["vector", "ANN", [1, 0]], limit: 2, include_attributes: ["vector"] },
          { rank_by: ["tokens", "BM25", ["fish"]], limit: 2 }
        ],
        rank_by: ["id", "desc"],
        filters: ["public", "Eq", 0],
        vector_encoding: "base64"
      }
    }
  ];
}

function isKnn(rankBy: JsonValue): boolean {
  return Array.isArray(rankBy) && rankBy[1] === "kNN";
}


function miniWrite(request: JsonObject): JsonObject {
  return parseJsonObject(
    micropuffer.write(namespaceName, JSON.stringify(request)),
    "micropuffer write response"
  );
}

function miniQuery(request: JsonObject): JsonObject {
  return parseJsonObject(
    micropuffer.query(namespaceName, JSON.stringify(request)),
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
