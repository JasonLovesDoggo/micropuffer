import { expect, test } from "vitest";
import {
  type JsonObject,
  type JsonValue,
  isJsonObject,
  parseJsonObject
} from "./test-utils";
import {
  Micropuffer
} from "micropuffer";

const namespaceName = "micropuffer-stateful-wasm-test";

test("stateful wasm engine owns query/write state in wasm memory", async () => {
  const engine = new Micropuffer();
  const ns = engine.namespace(namespaceName);
  const writeRequest: JsonObject = {
    distance_metric: "cosine_distance",
    schema: {
      title: { type: "string", full_text_search: true },
      tags: "[]string"
    },
    upsert_rows: [
      {
        id: 1,
        vector: [1, 0],
        title: "walrus field notes",
        tags: ["arctic", "mammal"],
        score: 10
      },
      {
        id: 2,
        vector: [0, 1],
        title: "reef fish field notes",
        tags: ["reef", "fish"],
        score: 4
      }
    ]
  };

  const write = expectJsonObject(await ns.write(writeRequest), "write response");
  expect(write.rows_upserted).toBe(2);

  const queryRequest: JsonObject = {
    rank_by: ["vector", "ANN", [1, 0]],
    limit: 1,
    include_attributes: ["title", "score"]
  };
  const query = expectJsonObject(await ns.query(queryRequest), "query response");
  expect(query.rows).toStrictEqual([{ "$dist": 0, id: 1, score: 10, title: "walrus field notes" }]);

  const metadata = expectJsonObject(await ns.metadata(), "metadata response");
  expect(metadata.approx_row_count).toBe(2);

  const exported = expectJsonObject(await ns.export({ limit: 10 }), "namespace export response");
  expect(exported.ids).toStrictEqual([1, 2]);
  expect(exported.vectors).toStrictEqual([[1, 0], [0, 1]]);
  expect(exported.attributes).toStrictEqual({
    score: [10, 4],
    tags: [["arctic", "mammal"], ["reef", "fish"]],
    title: ["walrus field notes", "reef fish field notes"]
  });

  const invalidProjection = expectJsonObject(
    await ns.queryResponse({ rank_by: ["id", "asc"], limit: 1, exclude_attributes: true }),
    "query response envelope"
  );
  expect(invalidProjection.status).toBe(422);
  expect(invalidProjection.body).toStrictEqual({
    status: "error",
    error: "Failed to deserialize the JSON body into the target type: invalid type: boolean `true`, expected a sequence"
  });

  const replacement = Micropuffer.fromStore(engine.exportStore());
  const replacementNs = replacement.namespace(namespaceName);
  expect(
    expectJsonObject(await replacementNs.query(queryRequest), "replacement query")
  ).toStrictEqual(expectJsonObject(await ns.query(queryRequest), "original query"));

  replacement.replaceStore(json({ namespaces: [] }));
  expect(parseJsonObject(replacement.listNamespaces(json({})), "empty namespace list")).toStrictEqual({
    namespaces: [],
    next_cursor: null
  });

  const firstPage = parseJsonObject(
    engine.listNamespaces(json({ prefix: namespaceName, page_size: "1" })),
    "string page size namespace list"
  );
  expect(firstPage.namespaces).toStrictEqual([{ id: namespaceName }]);
  expect(firstPage.next_cursor).toBeTypeOf("string");

  const invalidPageSize = parseJsonObject(
    engine.listNamespacesResponse(json({ page_size: "bad" })),
    "invalid page size namespace list"
  );
  expect(invalidPageSize).toStrictEqual({
    status: 400,
    body: "Failed to deserialize query string: invalid digit found in string"
  });
});

test("repeated stateful queries keep full store JSON out of the call loop", () => {
  const stateful = seedEngine(128);
  const request: JsonObject = {
    rank_by: ["score", "desc"],
    limit: 5,
    include_attributes: ["score"]
  };
  const requestJson = json(request);
  const storeJson = stateful.exportStore();
  const iterations = 100_000;

  for (let index = 0; index < 10; index += 1) {
    parseJsonObject(stateful.query(namespaceName, requestJson), "stateful repeated query");
  }

  const jsonRoundtripBytesPerQuery = storeJson.length + requestJson.length;
  const statefulBytesPerQuery = requestJson.length;
  expect(statefulBytesPerQuery).toBeLessThan(jsonRoundtripBytesPerQuery);
  expect((jsonRoundtripBytesPerQuery - statefulBytesPerQuery) * iterations).toBe(
    storeJson.length * iterations
  );
});

test("response methods expose HTTP-style status envelopes", async () => {
  const engine = new Micropuffer();
  const ns = engine.namespace("quickstart-response-test");

  const missingMetadata = parseJsonObject(
    engine.metadataResponse("missing"),
    "missing metadata response envelope"
  );
  expect(missingMetadata).toStrictEqual({
    status: 404,
    body: {
      status: "error",
      error: "🤷 namespace 'missing' was not found"
    }
  });

  const invalidNamespace = parseJsonObject(
    engine.queryResponse("bad namespace", json({ rank_by: ["id", "asc"], limit: 1 })),
    "invalid namespace response envelope"
  );
  expect(invalidNamespace).toStrictEqual({
    status: 400,
    body: "Invalid URL: Namespace contains invalid characters, must be [A-Za-z0-9-_.]"
  });

  const write = expectJsonObject(
    await ns.writeResponse({
      distance_metric: "cosine_distance",
      upsert_rows: [{ id: 1, vector: [1, 0] }]
    }),
    "write response envelope"
  );
  expect(write.status).toBe(200);
  expect(write.body).toMatchObject({ status: "OK", rows_affected: 1, rows_upserted: 1 });

  const warmCache = expectJsonObject(await ns.warmCacheResponse(), "warm cache response envelope");
  expect(warmCache).toStrictEqual({
    status: 202,
    body: {
      status: "ACCEPTED",
      message: "cache warm hint accepted"
    }
  });

  const deleteOk = expectJsonObject(await ns.deleteResponse(), "delete response envelope");
  expect(deleteOk).toStrictEqual({ status: 200, body: { status: "OK" } });

  const deleteMissing = expectJsonObject(await ns.deleteResponse(), "missing delete response envelope");
  expect(deleteMissing).toStrictEqual({
    status: 404,
    body: {
      status: "error",
      error: "🤷 namespace 'quickstart-response-test' was not found"
    }
  });
});

function seedEngine(rowCount: number): Micropuffer {
  const engine = new Micropuffer();
  engine.write(
    namespaceName,
    json({
      distance_metric: "cosine_distance",
      upsert_rows: Array.from({ length: rowCount }, (_, index) => ({
        id: index + 1,
        vector: [index + 1, rowCount - index],
        score: index
      }))
    })
  );
  return engine;
}

function json(value: JsonValue): string {
  return JSON.stringify(value);
}

function expectJsonObject(value: unknown, label: string): JsonObject {
  if (!isJsonObject(value)) {
    throw new Error(`${label} was not a JSON object.`);
  }
  return value;
}
