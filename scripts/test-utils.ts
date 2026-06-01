import dotenv from "dotenv";
import { expect } from "vitest";
import { z } from "zod";

export type JsonPrimitive = boolean | null | number | string;
export type JsonArray = JsonValue[];
export type JsonObject = { [key: string]: JsonValue };
export type JsonValue = JsonArray | JsonObject | JsonPrimitive;

export type MutationResult = {
  response: JsonObject;
  store: JsonObject;
};

export type ErrorResult = {
  status: number;
  body: JsonObject;
};

export const ENV_PATH = ".env";

const JsonValueSchema = z.json();
const JsonObjectSchema = z.record(z.string(), JsonValueSchema);
const MutationResultSchema = z.object({
  response: JsonObjectSchema,
  store: JsonObjectSchema
});

export function loadEnv(): void {
  dotenv.config({ path: ENV_PATH, quiet: true });
}

export function requiredEnv(name: string): string {
  const value = process.env[name];
  if (value === undefined || value.length === 0) {
    throw new Error(`${name} is required. Put it in ${ENV_PATH}.`);
  }
  return value;
}

export function envOrDefault(name: string, fallback: string): string {
  const value = process.env[name];
  return value === undefined || value.length === 0 ? fallback : value;
}

export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function integerEnv(name: string, fallback: number): number {
  const value = process.env[name];
  if (value === undefined || value.length === 0) {
    return fallback;
  }
  const parsed = Number.parseInt(value, 10);
  if (!Number.isInteger(parsed) || parsed <= 0) {
    throw new Error(`${name} must be a positive integer.`);
  }
  return parsed;
}

export function parseJsonObject(raw: string, label: string): JsonObject {
  const parsed = JsonObjectSchema.safeParse(JSON.parse(raw));
  if (!parsed.success) {
    throw new Error(`${label} was not a JSON object: ${z.prettifyError(parsed.error)}`);
  }
  return parsed.data;
}

export function parseMutationResult(raw: string, label: string): MutationResult {
  const parsed = MutationResultSchema.safeParse(JSON.parse(raw));
  if (!parsed.success) {
    throw new Error(`${label} did not contain response and store objects: ${z.prettifyError(parsed.error)}`);
  }
  return parsed.data;
}

export function isJsonObject(value: unknown): value is JsonObject {
  return JsonObjectSchema.safeParse(value).success;
}

export function stableJson(value: JsonValue): JsonValue {
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
  if (typeof value === "string") {
    return value.replace(".000000000Z", "Z");
  }
  return value;
}

export function expectJsonParity(live: JsonObject, local: JsonObject, message?: string): void {
  if (message === undefined) {
    expect(stableJson(local)).toStrictEqual(stableJson(live));
    return;
  }
  expect(stableJson(local), message).toStrictEqual(stableJson(live));
}

export class HttpError extends Error {
  readonly status: number;
  readonly body: string;

  constructor(status: number, body: string) {
    super(`turbopuffer request failed with HTTP ${status}: ${body}`);
    this.status = status;
    this.body = body;
  }
}
