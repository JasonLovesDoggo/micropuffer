import { readFile, writeFile } from "node:fs/promises";

const pkgDir = new URL("../pkg/", import.meta.url);
const generatedEntry = new URL("micropuffer.js", pkgDir);
const generatedTypes = new URL("micropuffer.d.ts", pkgDir);
const rawEntry = new URL("micropuffer_wasm.js", pkgDir);
const rawTypes = new URL("micropuffer_wasm.d.ts", pkgDir);

const generatedEntrySource = await readFile(generatedEntry, "utf8");
const generatedTypesSource = await readFile(generatedTypes, "utf8");

if (!generatedEntrySource.includes("./micropuffer_bg.js")) {
  throw new Error("pkg/micropuffer.js did not look like wasm-bindgen output.");
}

await writeFile(
  rawEntry,
  generatedEntrySource.replace(
    '@ts-self-types="./micropuffer.d.ts"',
    '@ts-self-types="./micropuffer_wasm.d.ts"',
  ),
);
await writeFile(rawTypes, generatedTypesSource);

await writeFile(
  generatedEntry,
  `import { Micropuffer as WasmMicropuffer } from "./micropuffer_wasm.js";

export class Micropuffer {
  #engine;

  constructor(engine) {
    this.#engine = engine ?? new WasmMicropuffer();
  }

  static fromStore(storeJson) {
    return new Micropuffer(WasmMicropuffer.fromStore(storeJson));
  }

  namespace(name) {
    return new Namespace(this, name);
  }

  free() {
    this.#engine.free();
  }

  [Symbol.dispose]() {
    const dispose = this.#engine[Symbol.dispose];
    if (typeof dispose === "function") {
      dispose.call(this.#engine);
      return;
    }
    this.free();
  }

  replaceStore(storeJson) {
    return this.#engine.replaceStore(storeJson);
  }

  exportStore() {
    return this.#engine.exportStore();
  }

  query(namespaceName, requestJson) {
    return this.#engine.query(namespaceName, requestJson);
  }

  queryResponse(namespaceName, requestJson) {
    return this.#engine.queryResponse(namespaceName, requestJson);
  }

  write(namespaceName, requestJson) {
    return this.#engine.write(namespaceName, requestJson);
  }

  writeResponse(namespaceName, requestJson) {
    return this.#engine.writeResponse(namespaceName, requestJson);
  }

  deleteNamespace(namespaceName) {
    return this.#engine.deleteNamespace(namespaceName);
  }

  deleteNamespaceResponse(namespaceName) {
    return this.#engine.deleteNamespaceResponse(namespaceName);
  }

  listNamespaces(requestJson) {
    return this.#engine.listNamespaces(requestJson);
  }

  listNamespacesResponse(requestJson) {
    return this.#engine.listNamespacesResponse(requestJson);
  }

  metadata(namespaceName) {
    return this.#engine.metadata(namespaceName);
  }

  metadataResponse(namespaceName) {
    return this.#engine.metadataResponse(namespaceName);
  }

  schema(namespaceName) {
    return this.#engine.schema(namespaceName);
  }

  schemaResponse(namespaceName) {
    return this.#engine.schemaResponse(namespaceName);
  }

  updateSchema(namespaceName, requestJson) {
    return this.#engine.updateSchema(namespaceName, requestJson);
  }

  updateSchemaResponse(namespaceName, requestJson) {
    return this.#engine.updateSchemaResponse(namespaceName, requestJson);
  }

  patchMetadata(namespaceName, requestJson) {
    return this.#engine.patchMetadata(namespaceName, requestJson);
  }

  patchMetadataResponse(namespaceName, requestJson) {
    return this.#engine.patchMetadataResponse(namespaceName, requestJson);
  }

  exportNamespace(namespaceName, requestJson) {
    return this.#engine.exportNamespace(namespaceName, requestJson);
  }

  exportNamespaceResponse(namespaceName, requestJson) {
    return this.#engine.exportNamespaceResponse(namespaceName, requestJson);
  }

  warmCache(namespaceName) {
    return this.#engine.warmCache(namespaceName);
  }

  warmCacheResponse(namespaceName) {
    return this.#engine.warmCacheResponse(namespaceName);
  }

  recall(namespaceName, requestJson) {
    return this.#engine.recall(namespaceName, requestJson);
  }

  recallResponse(namespaceName, requestJson) {
    return this.#engine.recallResponse(namespaceName, requestJson);
  }

  explainQuery(namespaceName, requestJson) {
    return this.#engine.explainQuery(namespaceName, requestJson);
  }

  explainQueryResponse(namespaceName, requestJson) {
    return this.#engine.explainQueryResponse(namespaceName, requestJson);
  }
}

export class Namespace {
  #engine;

  constructor(engine, name) {
    this.#engine = engine;
    this.name = name;
  }

  async query(request) {
    return parseJson(this.#engine.query(this.name, requestJson(request)), "query response");
  }

  async queryResponse(request) {
    return parseJson(this.#engine.queryResponse(this.name, requestJson(request)), "query response envelope");
  }

  async write(request) {
    return parseJson(this.#engine.write(this.name, requestJson(request)), "write response");
  }

  async writeResponse(request) {
    return parseJson(this.#engine.writeResponse(this.name, requestJson(request)), "write response envelope");
  }

  async delete() {
    return parseJson(this.#engine.deleteNamespace(this.name), "delete response");
  }

  async deleteResponse() {
    return parseJson(this.#engine.deleteNamespaceResponse(this.name), "delete response envelope");
  }

  async metadata() {
    return parseJson(this.#engine.metadata(this.name), "metadata response");
  }

  async metadataResponse() {
    return parseJson(this.#engine.metadataResponse(this.name), "metadata response envelope");
  }

  async schema() {
    return parseJson(this.#engine.schema(this.name), "schema response");
  }

  async schemaResponse() {
    return parseJson(this.#engine.schemaResponse(this.name), "schema response envelope");
  }

  async updateSchema(request) {
    return parseJson(this.#engine.updateSchema(this.name, requestJson(request)), "schema update response");
  }

  async updateSchemaResponse(request) {
    return parseJson(this.#engine.updateSchemaResponse(this.name, requestJson(request)), "schema update response envelope");
  }

  async patchMetadata(request) {
    return parseJson(this.#engine.patchMetadata(this.name, requestJson(request)), "metadata patch response");
  }

  async patchMetadataResponse(request) {
    return parseJson(this.#engine.patchMetadataResponse(this.name, requestJson(request)), "metadata patch response envelope");
  }

  async export(request = {}) {
    return parseJson(this.#engine.exportNamespace(this.name, requestJson(request)), "namespace export response");
  }

  async exportResponse(request = {}) {
    return parseJson(this.#engine.exportNamespaceResponse(this.name, requestJson(request)), "namespace export response envelope");
  }

  async warmCache() {
    return parseJson(this.#engine.warmCache(this.name), "warm cache response");
  }

  async warmCacheResponse() {
    return parseJson(this.#engine.warmCacheResponse(this.name), "warm cache response envelope");
  }

  async recall(request) {
    return parseJson(this.#engine.recall(this.name, requestJson(request)), "recall response");
  }

  async recallResponse(request) {
    return parseJson(this.#engine.recallResponse(this.name, requestJson(request)), "recall response envelope");
  }

  async explainQuery(request) {
    return parseJson(this.#engine.explainQuery(this.name, requestJson(request)), "explain query response");
  }

  async explainQueryResponse(request) {
    return parseJson(this.#engine.explainQueryResponse(this.name, requestJson(request)), "explain query response envelope");
  }
}

function requestJson(request) {
  if (typeof request === "string") {
    return request;
  }
  return JSON.stringify(request);
}

function parseJson(raw, label) {
  try {
    return JSON.parse(raw);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(\`\${label} was not valid JSON: \${message}\`);
  }
}
`,
);

await writeFile(
  generatedTypes,
  `export type JsonPrimitive = boolean | null | number | string;
export type JsonArray = JsonValue[];
export type JsonObject = { [key: string]: JsonValue };
export type JsonValue = JsonArray | JsonObject | JsonPrimitive;
export type RequestInput = JsonObject | string;

export interface HttpResponse {
  status: number;
  body: JsonValue;
}

export class Namespace {
  readonly name: string;
  query(request: RequestInput): Promise<JsonValue>;
  queryResponse(request: RequestInput): Promise<HttpResponse>;
  write(request: RequestInput): Promise<JsonValue>;
  writeResponse(request: RequestInput): Promise<HttpResponse>;
  delete(): Promise<JsonValue>;
  deleteResponse(): Promise<HttpResponse>;
  metadata(): Promise<JsonValue>;
  metadataResponse(): Promise<HttpResponse>;
  schema(): Promise<JsonValue>;
  schemaResponse(): Promise<HttpResponse>;
  updateSchema(request: RequestInput): Promise<JsonValue>;
  updateSchemaResponse(request: RequestInput): Promise<HttpResponse>;
  patchMetadata(request: RequestInput): Promise<JsonValue>;
  patchMetadataResponse(request: RequestInput): Promise<HttpResponse>;
  export(request?: RequestInput): Promise<JsonValue>;
  exportResponse(request?: RequestInput): Promise<HttpResponse>;
  warmCache(): Promise<JsonValue>;
  warmCacheResponse(): Promise<HttpResponse>;
  recall(request: RequestInput): Promise<JsonValue>;
  recallResponse(request: RequestInput): Promise<HttpResponse>;
  explainQuery(request: RequestInput): Promise<JsonValue>;
  explainQueryResponse(request: RequestInput): Promise<HttpResponse>;
}

export class Micropuffer {
  free(): void;
  [Symbol.dispose](): void;
  static fromStore(store_json: string): Micropuffer;
  constructor();
  namespace(name: string): Namespace;
  replaceStore(store_json: string): void;
  exportStore(): string;
  query(namespace_name: string, request_json: string): string;
  queryResponse(namespace_name: string, request_json: string): string;
  write(namespace_name: string, request_json: string): string;
  writeResponse(namespace_name: string, request_json: string): string;
  deleteNamespace(namespace_name: string): string;
  deleteNamespaceResponse(namespace_name: string): string;
  listNamespaces(request_json: string): string;
  listNamespacesResponse(request_json: string): string;
  metadata(namespace_name: string): string;
  metadataResponse(namespace_name: string): string;
  schema(namespace_name: string): string;
  schemaResponse(namespace_name: string): string;
  updateSchema(namespace_name: string, request_json: string): string;
  updateSchemaResponse(namespace_name: string, request_json: string): string;
  patchMetadata(namespace_name: string, request_json: string): string;
  patchMetadataResponse(namespace_name: string, request_json: string): string;
  exportNamespace(namespace_name: string, request_json: string): string;
  exportNamespaceResponse(namespace_name: string, request_json: string): string;
  warmCache(namespace_name: string): string;
  warmCacheResponse(namespace_name: string): string;
  recall(namespace_name: string, request_json: string): string;
  recallResponse(namespace_name: string, request_json: string): string;
  explainQuery(namespace_name: string, request_json: string): string;
  explainQueryResponse(namespace_name: string, request_json: string): string;
}
`,
);
