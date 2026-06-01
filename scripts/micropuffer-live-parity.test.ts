import { afterAll, expect, test } from "vitest";
import {
  type ErrorResult,
  type JsonObject,
  type JsonValue,
  HttpError,
  envOrDefault,
  expectJsonParity,
  isJsonObject,
  loadEnv,
  parseJsonObject,
  requiredEnv
} from "./test-utils";
import {
  Micropuffer
} from "micropuffer";


const NAMESPACE_PREFIX = "micropuffer-live-parity";
const namespaceName = `${NAMESPACE_PREFIX}-${Date.now()}-${process.pid}`;
const copyNamespaceName = `${namespaceName}-copy`;
const branchNamespaceName = `${namespaceName}-branch`;
const namespacesToDelete = [namespaceName, copyNamespaceName, branchNamespaceName];

loadEnv();

const apiKey = requiredEnv("TURBOPUFFER_API_KEY");
const region = envOrDefault("TURBOPUFFER_REGION", "gcp-us-central1");
const baseUrl = `https://${region}.turbopuffer.com`;

const micropuffer = new Micropuffer();

afterAll(async () => {
  for (const namespace of namespacesToDelete) {
    await deleteLiveNamespace(namespace);
  }
});

test("micropuffer wasm matches live turbopuffer for core query and workspace operations", async () => {
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

  expectJsonParity(await liveWrite(namespaceName, seedWrite), miniWrite(namespaceName, seedWrite));

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
  expectJsonParity(await liveQuery(namespaceName, vectorQuery), miniQuery(namespaceName, vectorQuery));

  const bm25Query: JsonObject = {
    rank_by: ["text", "BM25", "quick walrus"],
    limit: 10,
    filters: ["category", "Eq", "mammal"],
    include_attributes: ["text"]
  };
  expectJsonParity(await liveQuery(namespaceName, bm25Query), miniQuery(namespaceName, bm25Query));

  const sparseQuery: JsonObject = {
    rank_by: ["sparse_vector", "SparseKNN", { "0": 1.0, "2": 0.1 }],
    limit: 10,
    include_attributes: ["category"]
  };
  expectJsonParity(await liveQuery(namespaceName, sparseQuery), miniQuery(namespaceName, sparseQuery));

  const excludeAttributesQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 2,
    exclude_attributes: ["vector", "sparse_vector", "text"]
  };
  expectJsonParity(
    await liveQuery(namespaceName, excludeAttributesQuery),
    miniQuery(namespaceName, excludeAttributesQuery)
  );

  const includeAttributesFalseQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 2,
    include_attributes: false
  };
  expectJsonParity(
    await liveQuery(namespaceName, includeAttributesFalseQuery),
    miniQuery(namespaceName, includeAttributesFalseQuery)
  );

  const base64VectorQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    include_attributes: ["vector"],
    vector_encoding: "base64"
  };
  expectJsonParity(await liveQuery(namespaceName, base64VectorQuery), miniQuery(namespaceName, base64VectorQuery));

  const perCategoryOrderQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: {
      total: 3,
      per: { attributes: ["category"], limit: 1 }
    },
    include_attributes: ["category"]
  };
  expectJsonParity(
    await liveQuery(namespaceName, perCategoryOrderQuery),
    miniQuery(namespaceName, perCategoryOrderQuery)
  );

  const multiAttributeOrderQuery: JsonObject = {
    rank_by: [
      ["category", "asc"],
      ["score", "desc"]
    ],
    limit: 3,
    include_attributes: ["category", "score"]
  };
  expectJsonParity(
    await liveQuery(namespaceName, multiAttributeOrderQuery),
    miniQuery(namespaceName, multiAttributeOrderQuery)
  );

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
  expectJsonParity(await liveQuery(namespaceName, fuzzyQuery), miniQuery(namespaceName, fuzzyQuery));

  await assertColumnAndConditionParity();

  const aggregateQuery: JsonObject = {
    aggregate_by: { count: ["Count"] },
    group_by: [{ tag: ["ForEachUnique", "tags"] }],
    top_k: 10
  };
  expectJsonParity(await liveQuery(namespaceName, aggregateQuery), miniQuery(namespaceName, aggregateQuery));

  const aggregateDefaultLimitQuery: JsonObject = {
    aggregate_by: { count: ["Count"] },
    group_by: ["category"]
  };
  expectJsonParity(
    await liveQuery(namespaceName, aggregateDefaultLimitQuery),
    miniQuery(namespaceName, aggregateDefaultLimitQuery)
  );

  const sumAggregateQuery: JsonObject = {
    aggregate_by: { score_sum: ["Sum", "score"] }
  };
  expectJsonParity(await liveQuery(namespaceName, sumAggregateQuery), miniQuery(namespaceName, sumAggregateQuery));

  const multiQuery: JsonObject = {
    queries: [
      { rank_by: ["vector", "ANN", [1.0, 0.0]], limit: 1 },
      { rank_by: ["text", "BM25", "walrus"], limit: 2 }
    ]
  };
  expectJsonParity(await liveQuery(namespaceName, multiQuery), miniQuery(namespaceName, multiQuery));

  const patchByFilter: JsonObject = {
    patch_by_filter: {
      filters: ["category", "Eq", "fish"],
      patch: { category: "reef", score: 8 }
    },
    return_affected_ids: true
  };
  expectJsonParity(await liveWrite(namespaceName, patchByFilter), miniWrite(namespaceName, patchByFilter));

  const patchedLookup: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    include_attributes: ["category", "score"]
  };
  expectJsonParity(await liveQuery(namespaceName, patchedLookup), miniQuery(namespaceName, patchedLookup));

  const deleteByFilter: JsonObject = {
    delete_by_filter: ["score", "Lt", 6],
    return_affected_ids: true
  };
  expectJsonParity(await liveWrite(namespaceName, deleteByFilter), miniWrite(namespaceName, deleteByFilter));

  const afterDeleteLookup: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    include_attributes: ["category", "score"]
  };
  expectJsonParity(await liveQuery(namespaceName, afterDeleteLookup), miniQuery(namespaceName, afterDeleteLookup));

  await assertCopyParity(afterDeleteLookup);
  await assertDeprecatedExportParity();
  await assertListNamespaceParity();
  await assertMetadataParity();
  await assertSchemaParity();
  await assertSchemaUpdateParity();
  await assertWarmCacheParity();
  await assertRecallParity();
  await assertExplainQueryParity();
  await assertErrorParity();
  await assertBranchParityIfAllowed(afterDeleteLookup);
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
  expectJsonParity(await liveWrite(namespaceName, columnWrite), miniWrite(namespaceName, columnWrite));

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
  expectJsonParity(
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
  expectJsonParity(
    await liveWrite(namespaceName, conditionalPatch),
    miniWrite(namespaceName, conditionalPatch)
  );

  const conditionalDelete: JsonObject = {
    deletes: [5],
    delete_condition: ["score", "Gte", 20],
    return_affected_ids: true
  };
  expectJsonParity(
    await liveWrite(namespaceName, conditionalDelete),
    miniWrite(namespaceName, conditionalDelete)
  );

  const lookup: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    include_attributes: ["title", "score", "category"]
  };
  expectJsonParity(await liveQuery(namespaceName, lookup), miniQuery(namespaceName, lookup));
}

async function assertCopyParity(lookup: JsonObject): Promise<void> {
  const liveCopy = await liveWrite(copyNamespaceName, { copy_from_namespace: namespaceName });
  const miniCopy = miniWrite(copyNamespaceName, { copy_from_namespace: namespaceName });
  expect(miniCopy.rows_affected).toBe(liveCopy.rows_affected);
  expectJsonParity(await liveQuery(copyNamespaceName, lookup), miniQuery(copyNamespaceName, lookup));
}

async function assertDeprecatedExportParity(): Promise<void> {
  const live = await liveJson("GET", `/v1/namespaces/${encodeURIComponent(namespaceName)}`);
  expectJsonParity(live, miniExportNamespace(namespaceName));
}

async function assertListNamespaceParity(): Promise<void> {
  const live = await liveJson(
    "GET",
    `/v1/namespaces?prefix=${encodeURIComponent(namespaceName)}&page_size=10`
  );
  const mini = parseJsonObject(
    micropuffer.listNamespaces(JSON.stringify({ prefix: namespaceName, page_size: 10 })),
    "micropuffer list response"
  );
  expect(namespaceIds(mini)).toStrictEqual(namespaceIds(live));
}

async function assertMetadataParity(): Promise<void> {
  const live = await liveJson("GET", `/v1/namespaces/${encodeURIComponent(namespaceName)}/metadata`);
  const mini = parseJsonObject(
    micropuffer.metadata(namespaceName),
    "micropuffer metadata response"
  );
  const liveSchema = requireObject(live.schema, "live metadata schema");
  const miniSchema = requireObject(mini.schema, "micropuffer metadata schema");
  expect(mini.encryption).toStrictEqual(live.encryption);
  expect(requireObject(miniSchema.vector, "micropuffer metadata vector schema").ann).toStrictEqual(
    requireObject(liveSchema.vector, "live metadata vector schema").ann
  );
  expect(requireObject(miniSchema.category, "micropuffer metadata category schema").filterable).toBe(
    requireObject(liveSchema.category, "live metadata category schema").filterable
  );
  expect(requireObject(miniSchema.text, "micropuffer metadata text schema").full_text_search).toStrictEqual(
    requireObject(liveSchema.text, "live metadata text schema").full_text_search
  );
  expect(schemaTypes(mini)).toStrictEqual(schemaTypes(live));
}

async function assertSchemaParity(): Promise<void> {
  const live = await liveJson("GET", `/v1/namespaces/${encodeURIComponent(namespaceName)}/schema`);
  const mini = parseJsonObject(
    micropuffer.schema(namespaceName),
    "micropuffer schema response"
  );
  expect(requireObject(mini.id, "micropuffer id schema").filterable).toBe(
    requireObject(live.id, "live id schema").filterable
  );
  expect(requireObject(mini.id, "micropuffer id schema").full_text_search).toBe(
    requireObject(live.id, "live id schema").full_text_search
  );
  expect(requireObject(mini.vector, "micropuffer vector schema").ann).toStrictEqual(
    requireObject(live.vector, "live vector schema").ann
  );
  expect(requireObject(mini.category, "micropuffer category schema").filterable).toBe(
    requireObject(live.category, "live category schema").filterable
  );
  expect(requireObject(mini.text, "micropuffer text schema").full_text_search).toStrictEqual(
    requireObject(live.text, "live text schema").full_text_search
  );
  expect(schemaTypes({ schema: mini })).toStrictEqual(schemaTypes({ schema: live }));
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
  const mini = parseJsonObject(
    micropuffer.updateSchema(namespaceName, JSON.stringify(schemaUpdate)),
    "micropuffer schema update response"
  );
  expect(schemaTypes({ schema: mini })).toStrictEqual(schemaTypes({ schema: live }));

  const unknownSchemaOption: JsonObject = {
    category: {
      type: "string",
      regex: true,
      filterable: true,
      not_a_real_schema_option: true
    }
  };
  const liveUnknown = await liveJson(
    "POST",
    `/v1/namespaces/${encodeURIComponent(namespaceName)}/schema`,
    unknownSchemaOption
  );
  const miniUnknown = parseJsonObject(
    micropuffer.updateSchema(namespaceName, JSON.stringify(unknownSchemaOption)),
    "micropuffer schema update response"
  );
  expect(requireObject(miniUnknown.category, "micropuffer category schema")).toStrictEqual(
    requireObject(liveUnknown.category, "live category schema")
  );
  expect(requireObject(miniUnknown.category, "micropuffer category schema").not_a_real_schema_option)
    .toBeUndefined();

  const missingTypeSchemaOption: JsonObject = {
    category: {
      regex: true
    }
  };
  const liveMissingType = await liveError(
    "POST",
    `/v1/namespaces/${encodeURIComponent(namespaceName)}/schema`,
    missingTypeSchemaOption
  );
  const miniMissingType = miniUpdateSchemaError(namespaceName, missingTypeSchemaOption);
  expectErrorParity(miniMissingType, liveMissingType);

  const wrappedSchemaUpdate: JsonObject = {
    schema: {
      category: "string"
    }
  };
  const liveWrapped = await liveError(
    "POST",
    `/v1/namespaces/${encodeURIComponent(namespaceName)}/schema`,
    wrappedSchemaUpdate
  );
  const miniWrapped = miniUpdateSchemaError(namespaceName, wrappedSchemaUpdate);
  expectErrorParity(miniWrapped, liveWrapped);
}

async function assertWarmCacheParity(): Promise<void> {
  const live = await liveJson(
    "GET",
    `/v1/namespaces/${encodeURIComponent(namespaceName)}/hint_cache_warm`
  );
  const mini = parseJsonObject(
    micropuffer.warmCache(namespaceName),
    "micropuffer warm cache response"
  );
  expect(mini.status).toBe(live.status);
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
    micropuffer.recall(namespaceName, JSON.stringify(request)),
    "micropuffer recall response"
  );
  expect(typeof live.avg_recall).toBe("number");
  expect(typeof live.avg_exhaustive_count).toBe("number");
  expect(typeof live.avg_ann_count).toBe("number");
  expect(mini.avg_recall).toBe(1);
  expect(typeof mini.avg_exhaustive_count).toBe("number");
  expect(typeof mini.avg_ann_count).toBe("number");
}

async function assertExplainQueryParity(): Promise<void> {
  const request: JsonObject = {
    rank_by: ["text", "BM25", "walrus"],
    filters: ["public", "Eq", 1],
    limit: 3
  };
  const mini = parseJsonObject(
    micropuffer.explainQuery(namespaceName, JSON.stringify(request)),
    "micropuffer explain query response"
  );
  expect(typeof mini.plan_text).toBe("string");
  try {
    const live = await liveJson(
      "POST",
      `/v2/namespaces/${encodeURIComponent(namespaceName)}/explain_query`,
      request
    );
    expect(Object.keys(mini).sort()).toStrictEqual(Object.keys(live).sort());
  } catch (error) {
    if (
      error instanceof HttpError &&
      (error.status === 400 || error.status === 403 || error.status === 404)
    ) {
      console.info(`Skipping explain_query live shape parity: live endpoint returned HTTP ${error.status}.`);
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
  expectErrorParity(mini, live);

  const invalidPerQuery: JsonObject = {
    rank_by: ["text", "BM25", "walrus"],
    limit: {
      total: 3,
      per: { attributes: ["category"], limit: 1 }
    }
  };
  const livePer = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidPerQuery
  );
  const miniPer = miniQueryError(namespaceName, invalidPerQuery);
  expectErrorParity(miniPer, livePer);

  const invalidAggregateQuery: JsonObject = {
    aggregate_by: {
      count: ["Count"],
      score_sum: ["Sum", "score"]
    }
  };
  const liveAggregate = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidAggregateQuery
  );
  const miniAggregate = miniQueryError(namespaceName, invalidAggregateQuery);
  expectErrorParity(miniAggregate, liveAggregate);

  const invalidAggregateTopKQuery: JsonObject = {
    aggregate_by: {
      count: ["Count"]
    },
    top_k: 10
  };
  const liveAggregateTopK = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidAggregateTopKQuery
  );
  const miniAggregateTopK = miniQueryError(namespaceName, invalidAggregateTopKQuery);
  expectErrorParity(miniAggregateTopK, liveAggregateTopK);

  const invalidBm25ArrayQuery: JsonObject = {
    rank_by: ["text", "BM25", ["quick", "fish"]],
    limit: 10
  };
  const liveBm25Array = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidBm25ArrayQuery
  );
  const miniBm25Array = miniQueryError(namespaceName, invalidBm25ArrayQuery);
  expectErrorParity(miniBm25Array, liveBm25Array);

  const invalidExcludeAttributesQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    exclude_attributes: true
  };
  const liveExcludeAttributes = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidExcludeAttributesQuery
  );
  const miniExcludeAttributes = miniQueryError(namespaceName, invalidExcludeAttributesQuery);
  expectErrorParity(miniExcludeAttributes, liveExcludeAttributes);

  const invalidVectorEncodingQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    vector_encoding: "bad"
  };
  const liveVectorEncoding = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidVectorEncodingQuery
  );
  const miniVectorEncoding = miniQueryError(namespaceName, invalidVectorEncodingQuery);
  expectErrorParity(miniVectorEncoding, liveVectorEncoding);

  const missingNamespace = `${namespaceName}-missing`;
  const missingQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1
  };
  await deleteLiveNamespace(missingNamespace);
  const liveMissingNamespace = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(missingNamespace)}/query`,
    missingQuery
  );
  const miniMissingNamespace = miniQueryError(missingNamespace, missingQuery);
  expectErrorParity(miniMissingNamespace, liveMissingNamespace);

  const liveMissingDelete = await liveError(
    "DELETE",
    `/v2/namespaces/${encodeURIComponent(missingNamespace)}`
  );
  expectErrorParity(miniDeleteNamespaceError(missingNamespace), liveMissingDelete);

  const invalidSchemaUpdate: JsonObject = {
    category: "int"
  };
  const liveSchemaTypeChange = await liveError(
    "POST",
    `/v1/namespaces/${encodeURIComponent(namespaceName)}/schema`,
    invalidSchemaUpdate
  );
  const miniSchemaTypeChange = miniUpdateSchemaError(namespaceName, invalidSchemaUpdate);
  expectErrorParity(miniSchemaTypeChange, liveSchemaTypeChange);
  expect(miniSchemaTypeChange.body.attribute).toBe(liveSchemaTypeChange.body.attribute);

  const metricNamespace = `${namespaceName}-metric-error`;
  await deleteLiveNamespace(metricNamespace);
  const missingMetricWrite: JsonObject = {
    upsert_rows: [{ id: 1, vector: [1, 0] }]
  };
  const liveMissingMetric = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(metricNamespace)}`,
    missingMetricWrite
  );
  const miniMissingMetric = miniWriteError(metricNamespace, missingMetricWrite);
  expectErrorParity(miniMissingMetric, liveMissingMetric);

  const invalidMetricWrite: JsonObject = {
    distance_metric: "bad",
    upsert_rows: [{ id: 1, vector: [1, 0] }]
  };
  const liveInvalidMetric = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(metricNamespace)}`,
    invalidMetricWrite
  );
  const miniInvalidMetric = miniWriteError(metricNamespace, invalidMetricWrite);
  expectErrorParity(miniInvalidMetric, liveInvalidMetric);
  await deleteLiveNamespace(metricNamespace);

  const metricMismatchWrite: JsonObject = {
    distance_metric: "euclidean_squared",
    upsert_rows: [{ id: 99, vector: [0, 1] }]
  };
  const liveMetricMismatch = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}`,
    metricMismatchWrite
  );
  const miniMetricMismatch = miniWriteError(namespaceName, metricMismatchWrite);
  expectErrorParity(miniMetricMismatch, liveMetricMismatch);

  const scalarNamespace = `${namespaceName}-scalar-error`;
  await deleteLiveNamespace(scalarNamespace);
  const scalarSeed: JsonObject = {
    upsert_rows: [{ id: 1, title: "scalar" }]
  };
  expectJsonParity(await liveWrite(scalarNamespace, scalarSeed), miniWrite(scalarNamespace, scalarSeed));
  const addVectorToScalarWrite: JsonObject = {
    upsert_rows: [{ id: 2, vector: [1, 0], title: "vector" }]
  };
  const liveAddVectorToScalar = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(scalarNamespace)}`,
    addVectorToScalarWrite
  );
  const miniAddVectorToScalar = miniWriteError(scalarNamespace, addVectorToScalarWrite);
  expectErrorParity(miniAddVectorToScalar, liveAddVectorToScalar);
  await deleteLiveNamespace(scalarNamespace);

  const malformedRowsWrite: JsonObject = {
    upsert_rows: { id: 1 }
  };
  const liveMalformedRows = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(metricNamespace)}`,
    malformedRowsWrite
  );
  const miniMalformedRows = miniWriteError(metricNamespace, malformedRowsWrite);
  expectErrorParity(miniMalformedRows, liveMalformedRows);

  const missingIdWrite: JsonObject = {
    upsert_rows: [{ vector: [1, 0] }]
  };
  const liveMissingId = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(metricNamespace)}`,
    missingIdWrite
  );
  const miniMissingId = miniWriteError(metricNamespace, missingIdWrite);
  expectErrorParity(miniMissingId, liveMissingId);

  const malformedDeletesWrite: JsonObject = {
    deletes: true
  };
  const liveMalformedDeletes = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(metricNamespace)}`,
    malformedDeletesWrite
  );
  const miniMalformedDeletes = miniWriteError(metricNamespace, malformedDeletesWrite);
  expectErrorParity(miniMalformedDeletes, liveMalformedDeletes);

  const copyConflictWrite: JsonObject = {
    copy_from_namespace: namespaceName,
    upsert_rows: [{ id: 1 }]
  };
  const liveCopyConflict = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(metricNamespace)}-copy-conflict`,
    copyConflictWrite
  );
  const miniCopyConflict = miniWriteError(`${metricNamespace}-copy-conflict`, copyConflictWrite);
  expectErrorParity(miniCopyConflict, liveCopyConflict);
}

async function assertBranchParityIfAllowed(lookup: JsonObject): Promise<void> {
  try {
    await liveWrite(branchNamespaceName, { branch_from_namespace: namespaceName });
  } catch (error) {
    if (error instanceof HttpError && error.status === 403) {
      console.info("Skipping branch parity: API key is not permitted to branch namespaces.");
      return;
    }
    throw error;
  }
  miniWrite(branchNamespaceName, { branch_from_namespace: namespaceName });
  expectJsonParity(await liveQuery(branchNamespaceName, lookup), miniQuery(branchNamespaceName, lookup));
}

function miniWrite(namespace: string, request: JsonObject): JsonObject {
  return parseJsonObject(
    micropuffer.write(namespace, JSON.stringify(request)),
    "micropuffer write response"
  );
}

function miniQuery(namespace: string, request: JsonObject): JsonObject {
  return parseJsonObject(
    micropuffer.query(namespace, JSON.stringify(request)),
    "micropuffer query response"
  );
}

function miniExportNamespace(namespace: string): JsonObject {
  return parseJsonObject(
    micropuffer.exportNamespace(namespace, JSON.stringify({})),
    "micropuffer namespace export response"
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
  const response = parseJsonObject(
    micropuffer.queryResponse(namespace, JSON.stringify(request)),
    "micropuffer query response envelope"
  );
  return errorResultFromEnvelope(response, "query");
}

function miniWriteError(namespace: string, request: JsonObject): ErrorResult {
  const response = parseJsonObject(
    micropuffer.writeResponse(namespace, JSON.stringify(request)),
    "micropuffer write response envelope"
  );
  return errorResultFromEnvelope(response, "write");
}

function miniUpdateSchemaError(namespace: string, request: JsonObject): ErrorResult {
  const response = parseJsonObject(
    micropuffer.updateSchemaResponse(namespace, JSON.stringify(request)),
    "micropuffer update schema response envelope"
  );
  return errorResultFromEnvelope(response, "update schema");
}

function miniDeleteNamespaceError(namespace: string): ErrorResult {
  const response = parseJsonObject(
    micropuffer.deleteNamespaceResponse(namespace),
    "micropuffer delete namespace response envelope"
  );
  return errorResultFromEnvelope(response, "delete namespace");
}

function errorResultFromEnvelope(response: JsonObject, label: string): ErrorResult {
  const status = response.status;
  const body = response.body;
  if (typeof status !== "number") {
    throw new Error(`micropuffer ${label} response status was not a number.`);
  }
  if (!isJsonObject(body)) {
    throw new Error(`micropuffer ${label} response body was not an object.`);
  }
  if (status < 400) {
    throw new Error(`Expected micropuffer ${label} to fail.`);
  }
  return { status, body };
}

function expectErrorParity(mini: ErrorResult, live: ErrorResult): void {
  expect(mini.status).toBe(live.status);
  expect(mini.body.status).toBe(live.body.status);
  expect(errorWithoutLocation(mini.body.error)).toBe(errorWithoutLocation(live.body.error));
}

function errorWithoutLocation(error: JsonValue): string {
  if (typeof error !== "string") {
    throw new Error("error body did not contain a string error.");
  }
  return error.replace(/ at line \d+ column \d+$/, "");
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

function requireObject(value: JsonValue, label: string): JsonObject {
  if (!isJsonObject(value)) {
    throw new Error(`${label} was not a JSON object.`);
  }
  return value;
}
