import assert from "node:assert/strict";
import test from "node:test";

import { ApiError, createApiClient } from "../../src/api.ts";

function response(status: number, payload: unknown): Response {
  const body = JSON.stringify(payload);
  return new Response(body === undefined ? "" : body, {
    status,
    headers: { "content-type": "application/json" },
  });
}

test("client preserves request options and applies JSON content type", async () => {
  let received: RequestInit | undefined;
  const client = createApiClient({
    fetchImpl: async (_path, options) => {
      received = options;
      return response(200, { ok: true });
    },
  });

  assert.deepEqual(await client.send("/api/v1/status", "POST", { ready: true }), { ok: true });
  assert.ok(received);
  assert.equal(received.method, "POST");
  assert.equal(received.body, JSON.stringify({ ready: true }));
  assert.equal(new Headers(received.headers).get("Content-Type"), "application/json");
  assert.equal(received.cache, "no-store");
});

test("client preserves typed API errors and details", async () => {
  const client = createApiClient({
    fetchImpl: async () => response(403, {
      error: "arbitrary compatibility wording",
      code: "forbidden",
      message: "wording may change",
      details: { retry: true },
    }),
  });

  await assert.rejects(client.get("/api/v1/status"), (error) => {
    assert.ok(error instanceof ApiError);
    assert.equal(error.code, "forbidden");
    assert.deepEqual(error.details, { retry: true });
    return true;
  });
});

test("client maps non-JSON failures to a stable fallback", async () => {
  const client = createApiClient({
    fetchImpl: async () => new Response("not json", { status: 502 }),
  });
  await assert.rejects(client.get("/api"), /Request failed \(502\)/);
});
