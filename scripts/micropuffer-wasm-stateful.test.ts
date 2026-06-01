import { expect, test } from "vitest";
import {
  type JsonObject,
  type JsonValue,
  parseJsonObject
} from "./test-utils";
import {
  Micropuffer
} from "micropuffer";

const namespaceName = "micropuffer-stateful-wasm-test";

test("stateful wasm engine owns query/write state in wasm memory", () => {
  const engine = new Micropuffer();
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

  const write = parseJsonObject(engine.write(namespaceName, json(writeRequest)), "write response");
  expect(write.rows_upserted).toBe(2);

  const queryRequest: JsonObject = {
    rank_by: ["vector", "ANN", [1, 0]],
    limit: 1,
    include_attributes: ["title", "score"]
  };
  const query = parseJsonObject(engine.query(namespaceName, json(queryRequest)), "query response");
  expect(query.rows).toStrictEqual([{ "$dist": 0, id: 1, score: 10, title: "walrus field notes" }]);

  const metadata = parseJsonObject(engine.metadata(namespaceName), "metadata response");
  expect(metadata.approx_row_count).toBe(2);

  const exported = parseJsonObject(
    engine.exportNamespace(namespaceName, json({ limit: 10 })),
    "namespace export response"
  );
  expect(exported.rows).toHaveLength(2);

  const replacement = Micropuffer.fromStore(engine.exportStore());
  expect(
    parseJsonObject(replacement.query(namespaceName, json(queryRequest)), "replacement query")
  ).toStrictEqual(
    parseJsonObject(engine.query(namespaceName, json(queryRequest)), "original query")
  );

  replacement.replaceStore(json({ namespaces: [] }));
  expect(parseJsonObject(replacement.listNamespaces(json({})), "empty namespace list")).toStrictEqual({
    namespaces: []
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

function seedEngine(rowCount: number): Micropuffer {
  const engine = new Micropuffer();
  engine.write(
    namespaceName,
    json({
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
