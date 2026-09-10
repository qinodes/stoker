import assert from "node:assert/strict";
import test from "node:test";

import { ApiError, createApiClient } from "../../src/api.ts";

function response(status: number, payload: unknown) {
  return {
    status,
    ok: status >= 200 && status < 300,
    json: async () => payload,
  };
}

test("client stores fragment token and sends it as a bearer header", async () => {
  const values = new Map<string, string>();
  const session = {
    getItem: (key: string) => values.get(key) || null,
    setItem: (key: string, value: string) => values.set(key, value),
  };
  let received: any;
  const client = createApiClient({
    fetchImpl: async (_path, options) => {
      received = options;
      return response(200, { ok: true });
    },
    session,
    browserLocation: { hash: "#token=secret", pathname: "/", search: "" } as Location,
    browserHistory: { replaceState() {} } as History,
  });

  assert.deepEqual(await client.get("/api/v1/status"), { ok: true });
  assert.equal(values.get("stoker-ui-token"), "secret");
  assert.equal(received.headers.get("Authorization"), "Bearer secret");
  assert.equal(received.cache, "no-store");
});

test("client branches on typed unauthorized code instead of message text", async () => {
  let unauthorized = 0;
  const client = createApiClient({
    fetchImpl: async () => response(403, {
      error: "arbitrary compatibility wording",
      code: "unauthorized",
      message: "wording may change",
      details: { retry: true },
    }),
    session: { getItem: () => null, setItem() {} },
    browserLocation: { hash: "", pathname: "/", search: "" } as Location,
    browserHistory: { replaceState() {} } as History,
    onUnauthorized: () => { unauthorized += 1; },
  });

  await assert.rejects(client.get("/api/v1/status"), (error) => {
    assert.ok(error instanceof ApiError);
    assert.equal(error.code, "unauthorized");
    assert.deepEqual(error.details, { retry: true });
    return true;
  });
  assert.equal(unauthorized, 1);
});

test("client maps non-JSON failures to a stable fallback", async () => {
  const client = createApiClient({
    fetchImpl: async () => ({ status: 502, ok: false, json: async () => { throw new Error("not json"); } }),
    session: { getItem: () => null, setItem() {} },
    browserLocation: { hash: "", pathname: "/", search: "" } as Location,
    browserHistory: { replaceState() {} } as History,
  });
  await assert.rejects(client.get("/api"), /Request failed \(502\)/);
});
