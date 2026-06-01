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
const euclideanNamespaceName = `${namespaceName}-euclidean`;
const base64NamespaceName = `${namespaceName}-base64`;
const uuidNamespaceName = `${namespaceName}-uuid`;
const uuidInvalidNamespaceName = `${namespaceName}-uuid-invalid`;
const stringIdNamespaceName = `${namespaceName}-id-string`;
const uintIdNamespaceName = `${namespaceName}-id-uint`;
const inferredUintIdNamespaceName = `${namespaceName}-id-inferred-uint`;
const invalidIdSchemaNamespaceName = `${namespaceName}-id-invalid-schema`;
const namespacesToDelete = [
  namespaceName,
  copyNamespaceName,
  branchNamespaceName,
  euclideanNamespaceName,
  base64NamespaceName,
  uuidNamespaceName,
  uuidInvalidNamespaceName,
  stringIdNamespaceName,
  uintIdNamespaceName,
  inferredUintIdNamespaceName,
  invalidIdSchemaNamespaceName
];

loadEnv();

const apiKey = requiredEnv("TURBOPUFFER_API_KEY");
const region = envOrDefault("TURBOPUFFER_REGION", "gcp-us-central1");
const baseUrl = `https://${region}.turbopuffer.com`;

const micropuffer = new Micropuffer();

type TextErrorResult = {
  status: number;
  body: string;
};

afterAll(async () => {
  for (const namespace of namespacesToDelete) {
    await deleteLiveNamespace(namespace);
  }
});

test("micropuffer wasm matches live turbopuffer for core query and workspace operations", async () => {
  await deleteLiveNamespace(namespaceName);
  await deleteLiveNamespace(copyNamespaceName);
  await deleteLiveNamespace(branchNamespaceName);
  await deleteLiveNamespace(base64NamespaceName);
  await deleteLiveNamespace(uuidNamespaceName);
  await deleteLiveNamespace(uuidInvalidNamespaceName);
  await deleteLiveNamespace(stringIdNamespaceName);
  await deleteLiveNamespace(uintIdNamespaceName);
  await deleteLiveNamespace(inferredUintIdNamespaceName);
  await deleteLiveNamespace(invalidIdSchemaNamespaceName);

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

  const knnQuery: JsonObject = {
    rank_by: ["vector", "kNN", [1.0, 0.0]],
    limit: 2,
    filters: ["public", "Eq", 1],
    include_attributes: ["category", "score"]
  };
  expectJsonParity(await liveQuery(namespaceName, knnQuery), miniQuery(namespaceName, knnQuery));

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

  const includeAttributesTrueQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    include_attributes: true
  };
  expectJsonParity(
    await liveQuery(namespaceName, includeAttributesTrueQuery),
    miniQuery(namespaceName, includeAttributesTrueQuery)
  );

  const includeAttributesEmptyQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    include_attributes: []
  };
  expectJsonParity(
    await liveQuery(namespaceName, includeAttributesEmptyQuery),
    miniQuery(namespaceName, includeAttributesEmptyQuery)
  );

  const eventualConsistencyQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    consistency: { level: "eventual" }
  };
  expectJsonParity(
    await liveQuery(namespaceName, eventualConsistencyQuery),
    miniQuery(namespaceName, eventualConsistencyQuery)
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
  await assertInvalidNamespaceParity();
  await assertEuclideanDistanceMetricParity();
  await assertBase64VectorInputParity();
  await assertUuidIdSchemaParity();
  await assertTypedIdSchemaParity();
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
    micropuffer.listNamespaces(JSON.stringify({ prefix: namespaceName, page_size: "10" })),
    "micropuffer list response"
  );
  expectJsonParity(live, mini);

  const liveFirstPage = await liveJson(
    "GET",
    `/v1/namespaces?prefix=${encodeURIComponent(namespaceName)}&page_size=1`
  );
  const miniFirstPage = parseJsonObject(
    micropuffer.listNamespaces(JSON.stringify({ prefix: namespaceName, page_size: "1" })),
    "micropuffer list first page response"
  );
  expectJsonParity(liveFirstPage, miniFirstPage);
  const cursor = requireString(liveFirstPage.next_cursor, "live namespace list cursor");
  const liveSecondPage = await liveJson(
    "GET",
    `/v1/namespaces?prefix=${encodeURIComponent(namespaceName)}&page_size=1&cursor=${encodeURIComponent(cursor)}`
  );
  const miniSecondPage = parseJsonObject(
    micropuffer.listNamespaces(JSON.stringify({ prefix: namespaceName, page_size: "1", cursor })),
    "micropuffer list second page response"
  );
  expectJsonParity(liveSecondPage, miniSecondPage);

  const liveEmptyPage = await liveJson(
    "GET",
    `/v1/namespaces?prefix=${encodeURIComponent(`${namespaceName}-empty`)}&page_size=1`
  );
  const miniEmptyPage = parseJsonObject(
    micropuffer.listNamespaces(JSON.stringify({ prefix: `${namespaceName}-empty`, page_size: "1" })),
    "micropuffer list empty page response"
  );
  expectJsonParity(liveEmptyPage, miniEmptyPage);

  const liveZeroPageSize = await liveError(
    "GET",
    `/v1/namespaces?prefix=${encodeURIComponent(namespaceName)}&page_size=0`
  );
  const miniZeroPageSize = miniListNamespacesJsonError({ prefix: namespaceName, page_size: "0" });
  expectErrorParity(miniZeroPageSize, liveZeroPageSize);

  const liveBadPageSize = await liveTextError(
    "GET",
    `/v1/namespaces?prefix=${encodeURIComponent(namespaceName)}&page_size=bad`
  );
  const miniBadPageSize = miniListNamespacesTextError({ prefix: namespaceName, page_size: "bad" });
  expectTextErrorParity(miniBadPageSize, liveBadPageSize);
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
  expectJsonParity(live, mini);
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

  const missingIncludeAttributeQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    include_attributes: ["missing_attr"]
  };
  const liveMissingIncludeAttribute = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    missingIncludeAttributeQuery
  );
  const miniMissingIncludeAttribute = miniQueryError(namespaceName, missingIncludeAttributeQuery);
  expectErrorParity(miniMissingIncludeAttribute, liveMissingIncludeAttribute);

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

  const invalidConsistencyLevelQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    consistency: { level: "bad" }
  };
  const liveInvalidConsistencyLevel = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidConsistencyLevelQuery
  );
  const miniInvalidConsistencyLevel = miniQueryError(namespaceName, invalidConsistencyLevelQuery);
  expectErrorParity(miniInvalidConsistencyLevel, liveInvalidConsistencyLevel);

  const invalidConsistencyShapeQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    consistency: "eventual"
  };
  const liveInvalidConsistencyShape = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidConsistencyShapeQuery
  );
  const miniInvalidConsistencyShape = miniQueryError(namespaceName, invalidConsistencyShapeQuery);
  expectErrorParity(miniInvalidConsistencyShape, liveInvalidConsistencyShape);

  const missingConsistencyLevelQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    consistency: {}
  };
  const liveMissingConsistencyLevel = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    missingConsistencyLevelQuery
  );
  const miniMissingConsistencyLevel = miniQueryError(namespaceName, missingConsistencyLevelQuery);
  expectErrorParity(miniMissingConsistencyLevel, liveMissingConsistencyLevel);

  const invalidConsistencyLevelTypeQuery: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1,
    consistency: { level: 1 }
  };
  const liveInvalidConsistencyLevelType = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(namespaceName)}/query`,
    invalidConsistencyLevelTypeQuery
  );
  const miniInvalidConsistencyLevelType = miniQueryError(namespaceName, invalidConsistencyLevelTypeQuery);
  expectErrorParity(miniInvalidConsistencyLevelType, liveInvalidConsistencyLevelType);

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

  const schemaAnnMetricWrite: JsonObject = {
    schema: {
      vector: {
        type: "[2]f32",
        ann: { distance_metric: "euclidean" }
      }
    },
    upsert_rows: [{ id: 1, vector: [1, 0] }]
  };
  const liveSchemaAnnMetric = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(metricNamespace)}`,
    schemaAnnMetricWrite
  );
  const miniSchemaAnnMetric = miniWriteError(metricNamespace, schemaAnnMetricWrite);
  expectErrorParity(miniSchemaAnnMetric, liveSchemaAnnMetric);

  const missingAnnWrite: JsonObject = {
    distance_metric: "cosine_distance",
    schema: { vector: "[2]f32" },
    upsert_rows: [{ id: 1, vector: [1, 0] }]
  };
  const liveMissingAnn = await liveError(
    "POST",
    `/v2/namespaces/${encodeURIComponent(metricNamespace)}`,
    missingAnnWrite
  );
  const miniMissingAnn = miniWriteError(metricNamespace, missingAnnWrite);
  expectErrorParity(miniMissingAnn, liveMissingAnn);

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

  const malformedIdWrites: JsonObject[] = [
    { upsert_rows: [{ id: false }] },
    { patch_rows: [{ id: 1.5 }] },
    { upsert_columns: { id: [true], title: ["bad"] } },
    { patch_columns: { id: [1.5], title: ["bad"] } },
    { deletes: [true] }
  ];
  for (const malformedIdWrite of malformedIdWrites) {
    expectErrorParity(
      miniWriteError(metricNamespace, malformedIdWrite),
      await liveError("POST", `/v2/namespaces/${encodeURIComponent(metricNamespace)}`, malformedIdWrite)
    );
  }

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

async function assertInvalidNamespaceParity(): Promise<void> {
  const query: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 1
  };
  const cases: Array<{ namespace: string }> = [
    { namespace: "bad namespace" },
    { namespace: "a".repeat(129) }
  ];

  for (const testCase of cases) {
    const live = await liveText(
      "POST",
      `/v2/namespaces/${encodeURIComponent(testCase.namespace)}/query`,
      query
    );
    const mini = parseJsonObject(
      micropuffer.queryResponse(testCase.namespace, JSON.stringify(query)),
      "micropuffer invalid namespace response envelope"
    );
    expect(mini.status).toBe(live.status);
    expect(mini.body).toBe(live.body);
  }
}

async function assertEuclideanDistanceMetricParity(): Promise<void> {
  await deleteLiveNamespace(euclideanNamespaceName);
  const write: JsonObject = {
    distance_metric: "euclidean",
    upsert_rows: [
      { id: 1, vector: [1, 0] },
      { id: 2, vector: [0, 2] }
    ]
  };
  expectJsonParity(await liveWrite(euclideanNamespaceName, write), miniWrite(euclideanNamespaceName, write));

  const query: JsonObject = {
    rank_by: ["vector", "ANN", [0, 0]],
    limit: 2,
    include_attributes: ["vector"]
  };
  expectJsonParity(await liveQuery(euclideanNamespaceName, query), miniQuery(euclideanNamespaceName, query));

  const liveMetadata = await liveJson(
    "GET",
    `/v1/namespaces/${encodeURIComponent(euclideanNamespaceName)}/metadata`
  );
  const miniMetadata = parseJsonObject(
    micropuffer.metadata(euclideanNamespaceName),
    "micropuffer euclidean metadata response"
  );
  expect(
    requireObject(requireObject(liveMetadata.schema, "live euclidean metadata schema").vector, "live vector schema")
      .ann
  ).toStrictEqual(
    requireObject(
      requireObject(miniMetadata.schema, "micropuffer euclidean metadata schema").vector,
      "micropuffer vector schema"
    ).ann
  );
}

async function assertBase64VectorInputParity(): Promise<void> {
  await deleteLiveNamespace(base64NamespaceName);
  const encodedVector = float32Base64([1, 0]);
  const write: JsonObject = {
    distance_metric: "cosine_distance",
    schema: { vector: { type: "[2]f32", ann: true } },
    upsert_rows: [{ id: 1, vector: encodedVector, title: "base64" }],
    return_affected_ids: true
  };
  expectJsonParity(await liveWrite(base64NamespaceName, write), miniWrite(base64NamespaceName, write));

  const query: JsonObject = {
    rank_by: ["vector", "ANN", encodedVector],
    limit: 1,
    include_attributes: ["vector"],
    vector_encoding: "base64"
  };
  expectJsonParity(await liveQuery(base64NamespaceName, query), miniQuery(base64NamespaceName, query));
}

async function assertUuidIdSchemaParity(): Promise<void> {
  await deleteLiveNamespace(uuidNamespaceName);
  await deleteLiveNamespace(uuidInvalidNamespaceName);

  const invalidFirstWrite: JsonObject = {
    schema: { id: "uuid", title: "string" },
    upsert_rows: [{ id: "not-a-uuid", title: "bad" }]
  };
  expectErrorParity(
    miniWriteError(uuidInvalidNamespaceName, invalidFirstWrite),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(uuidInvalidNamespaceName)}`, invalidFirstWrite)
  );

  const write: JsonObject = {
    schema: { id: "uuid", title: "string" },
    upsert_rows: [
      { id: "769C134D-07B8-4225-954A-B6CC5FFC320C", title: "row-0" },
      { id: "769c134d07b84225954ab6cc5ffc320d", title: "row-1" },
      { id: "{769c134d-07b8-4225-954a-b6cc5ffc320e}", title: "row-2" },
      { id: "urn:uuid:769c134d-07b8-4225-954a-b6cc5ffc320f", title: "row-3" }
    ],
    return_affected_ids: true
  };
  expectJsonParity(await liveWrite(uuidNamespaceName, write), miniWrite(uuidNamespaceName, write));

  const lookup: JsonObject = {
    rank_by: ["title", "asc"],
    limit: 10,
    include_attributes: ["title"]
  };
  expectJsonParity(await liveQuery(uuidNamespaceName, lookup), miniQuery(uuidNamespaceName, lookup));

  const uppercaseIdFilter: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    filters: ["id", "Eq", "769C134D-07B8-4225-954A-B6CC5FFC320C"]
  };
  expectJsonParity(
    await liveQuery(uuidNamespaceName, uppercaseIdFilter),
    miniQuery(uuidNamespaceName, uppercaseIdFilter)
  );

  const simpleIdFilter: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    filters: ["id", "Eq", "769c134d07b84225954ab6cc5ffc320c"]
  };
  expectJsonParity(await liveQuery(uuidNamespaceName, simpleIdFilter), miniQuery(uuidNamespaceName, simpleIdFilter));

  const uuidRangeFilter: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    filters: ["id", "Gte", "769c134d07b84225954ab6cc5ffc320c"]
  };
  expectJsonParity(await liveQuery(uuidNamespaceName, uuidRangeFilter), miniQuery(uuidNamespaceName, uuidRangeFilter));

  const invalidUuidFilter: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    filters: ["id", "Eq", "not-a-uuid"]
  };
  expectErrorParity(
    miniQueryError(uuidNamespaceName, invalidUuidFilter),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(uuidNamespaceName)}/query`, invalidUuidFilter)
  );

  const invalidUuidInFilter: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    filters: [
      "id",
      "In",
      ["not-a-uuid", "769C134D-07B8-4225-954A-B6CC5FFC320C"]
    ]
  };
  expectErrorParity(
    miniQueryError(uuidNamespaceName, invalidUuidInFilter),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(uuidNamespaceName)}/query`, invalidUuidInFilter)
  );

  const liveSchema = await liveJson("GET", `/v1/namespaces/${encodeURIComponent(uuidNamespaceName)}/schema`);
  const miniSchema = parseJsonObject(
    micropuffer.schema(uuidNamespaceName),
    "micropuffer uuid schema response"
  );
  expect(requireObject(miniSchema.id, "micropuffer uuid id schema").type).toBe(
    requireObject(liveSchema.id, "live uuid id schema").type
  );

  const patchRows: JsonObject = {
    patch_rows: [
      { id: "769C134D-07B8-4225-954A-B6CC5FFC320C", title: "patched" }
    ],
    return_affected_ids: true
  };
  expectJsonParity(await liveWrite(uuidNamespaceName, patchRows), miniWrite(uuidNamespaceName, patchRows));

  const patchColumns: JsonObject = {
    patch_columns: {
      id: ["769c134d07b84225954ab6cc5ffc320d"],
      title: ["column patched"]
    },
    return_affected_ids: true
  };
  expectJsonParity(await liveWrite(uuidNamespaceName, patchColumns), miniWrite(uuidNamespaceName, patchColumns));

  const conditionalUuidUpsert: JsonObject = {
    upsert_rows: [
      { id: "769C134D-07B8-4225-954A-B6CC5FFC320C", title: "conditioned" }
    ],
    upsert_condition: ["id", "Eq", "769C134D-07B8-4225-954A-B6CC5FFC320C"],
    return_affected_ids: true
  };
  expectJsonParity(
    await liveWrite(uuidNamespaceName, conditionalUuidUpsert),
    miniWrite(uuidNamespaceName, conditionalUuidUpsert)
  );

  const conditionalUuidPatch: JsonObject = {
    patch_rows: [
      { id: "769c134d07b84225954ab6cc5ffc320d", title: "conditioned patch" }
    ],
    patch_condition: ["id", "Eq", "769c134d07b84225954ab6cc5ffc320d"],
    return_affected_ids: true
  };
  expectJsonParity(
    await liveWrite(uuidNamespaceName, conditionalUuidPatch),
    miniWrite(uuidNamespaceName, conditionalUuidPatch)
  );

  const conditionalUuidDelete: JsonObject = {
    deletes: ["{769c134d-07b8-4225-954a-b6cc5ffc320e}"],
    delete_condition: ["id", "Eq", "{769c134d-07b8-4225-954a-b6cc5ffc320e}"],
    return_affected_ids: true
  };
  expectJsonParity(
    await liveWrite(uuidNamespaceName, conditionalUuidDelete),
    miniWrite(uuidNamespaceName, conditionalUuidDelete)
  );

  const deleteByUrn: JsonObject = {
    deletes: ["urn:uuid:769c134d-07b8-4225-954a-b6cc5ffc320f"],
    return_affected_ids: true
  };
  expectJsonParity(await liveWrite(uuidNamespaceName, deleteByUrn), miniWrite(uuidNamespaceName, deleteByUrn));

  const invalidDelete: JsonObject = { deletes: ["not-a-uuid"] };
  expectErrorParity(
    miniWriteError(uuidNamespaceName, invalidDelete),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(uuidNamespaceName)}`, invalidDelete)
  );

  const invalidPatchColumns: JsonObject = {
    patch_columns: {
      id: ["urn:uuid:{769c134d-07b8-4225-954a-b6cc5ffc320c}"],
      title: ["bad"]
    }
  };
  expectErrorParity(
    miniWriteError(uuidNamespaceName, invalidPatchColumns),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(uuidNamespaceName)}`, invalidPatchColumns)
  );

  const duplicateUuidWrite: JsonObject = {
    upsert_rows: [
      { id: "769C134D-07B8-4225-954A-B6CC5FFC320C", title: "dupe-0" },
      { id: "769c134d-07b8-4225-954a-b6cc5ffc320c", title: "dupe-1" }
    ]
  };
  expectErrorParity(
    miniWriteError(uuidNamespaceName, duplicateUuidWrite),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(uuidNamespaceName)}`, duplicateUuidWrite)
  );
}

async function assertTypedIdSchemaParity(): Promise<void> {
  await deleteLiveNamespace(stringIdNamespaceName);
  await deleteLiveNamespace(uintIdNamespaceName);
  await deleteLiveNamespace(inferredUintIdNamespaceName);
  await deleteLiveNamespace(invalidIdSchemaNamespaceName);

  const invalidIdSchema: JsonObject = {
    schema: { id: "int", title: "string" },
    upsert_rows: [{ id: 1, title: "bad" }]
  };
  expectErrorParity(
    miniWriteError(invalidIdSchemaNamespaceName, invalidIdSchema),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(invalidIdSchemaNamespaceName)}`, invalidIdSchema)
  );

  const stringIdMismatch: JsonObject = {
    schema: { id: "string", title: "string" },
    upsert_rows: [{ id: 1, title: "bad" }]
  };
  expectErrorParity(
    miniWriteError(stringIdNamespaceName, stringIdMismatch),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(stringIdNamespaceName)}`, stringIdMismatch)
  );

  const stringIdWrite: JsonObject = {
    schema: { id: "string", title: "string" },
    upsert_rows: [{ id: "abc", title: "good" }]
  };
  expectJsonParity(await liveWrite(stringIdNamespaceName, stringIdWrite), miniWrite(stringIdNamespaceName, stringIdWrite));

  const stringIdFilterMismatch: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    filters: ["id", "Eq", 1]
  };
  expectErrorParity(
    miniQueryError(stringIdNamespaceName, stringIdFilterMismatch),
    await liveError(
      "POST",
      `/v2/namespaces/${encodeURIComponent(stringIdNamespaceName)}/query`,
      stringIdFilterMismatch
    )
  );

  const uintIdMismatch: JsonObject = {
    schema: { id: "uint", title: "string" },
    upsert_rows: [{ id: "abc", title: "bad" }]
  };
  expectErrorParity(
    miniWriteError(uintIdNamespaceName, uintIdMismatch),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(uintIdNamespaceName)}`, uintIdMismatch)
  );

  const uintIdWrite: JsonObject = {
    schema: { id: "uint", title: "string" },
    upsert_rows: [{ id: 1, title: "good" }]
  };
  expectJsonParity(await liveWrite(uintIdNamespaceName, uintIdWrite), miniWrite(uintIdNamespaceName, uintIdWrite));

  const uintIdFilterMismatch: JsonObject = {
    rank_by: ["id", "asc"],
    limit: 10,
    filters: ["id", "Eq", "abc"]
  };
  expectErrorParity(
    miniQueryError(uintIdNamespaceName, uintIdFilterMismatch),
    await liveError("POST", `/v2/namespaces/${encodeURIComponent(uintIdNamespaceName)}/query`, uintIdFilterMismatch)
  );

  const inferredUintWrite: JsonObject = {
    upsert_rows: [{ id: 1, title: "good" }]
  };
  expectJsonParity(
    await liveWrite(inferredUintIdNamespaceName, inferredUintWrite),
    miniWrite(inferredUintIdNamespaceName, inferredUintWrite)
  );

  const inferredUintMismatch: JsonObject = {
    upsert_rows: [{ id: "1", title: "bad" }]
  };
  expectErrorParity(
    miniWriteError(inferredUintIdNamespaceName, inferredUintMismatch),
    await liveError(
      "POST",
      `/v2/namespaces/${encodeURIComponent(inferredUintIdNamespaceName)}`,
      inferredUintMismatch
    )
  );
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

async function liveTextError(
  method: "DELETE" | "GET" | "POST",
  path: string,
  body?: JsonObject
): Promise<TextErrorResult> {
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
    body: responseBody
  };
}

function miniListNamespacesJsonError(request: JsonObject): ErrorResult {
  const response = parseJsonObject(
    micropuffer.listNamespacesResponse(JSON.stringify(request)),
    "micropuffer list namespaces response envelope"
  );
  return errorResultFromEnvelope(response, "list namespaces");
}

function miniListNamespacesTextError(request: JsonObject): TextErrorResult {
  const response = parseJsonObject(
    micropuffer.listNamespacesResponse(JSON.stringify(request)),
    "micropuffer list namespaces response envelope"
  );
  const status = response.status;
  const body = response.body;
  if (typeof status !== "number") {
    throw new Error("micropuffer list namespaces response status was not a number.");
  }
  if (typeof body !== "string") {
    throw new Error("micropuffer list namespaces response body was not a string.");
  }
  if (status < 400) {
    throw new Error("Expected micropuffer list namespaces to fail.");
  }
  return { status, body };
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

function expectTextErrorParity(mini: TextErrorResult, live: TextErrorResult): void {
  expect(mini.status).toBe(live.status);
  expect(mini.body).toBe(live.body);
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

async function liveText(
  method: "DELETE" | "GET" | "POST",
  path: string,
  body?: JsonObject
): Promise<{ status: number; body: string }> {
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
  return { status: response.status, body: await response.text() };
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

function float32Base64(values: number[]): string {
  const buffer = Buffer.alloc(values.length * 4);
  values.forEach((value, index) => buffer.writeFloatLE(value, index * 4));
  return buffer.toString("base64");
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

function requireString(value: JsonValue, label: string): string {
  if (typeof value !== "string") {
    throw new Error(`${label} was not a string.`);
  }
  return value;
}
