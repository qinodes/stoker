import assert from "node:assert/strict";
import test from "node:test";

import { ApiError, createApiClient } from "../../modules/api-client.js";

function response(status, payload) {
  return {
    status,
    ok: status >= 200 && status < 300,
    json: async () => payload,
  };
}

test("client stores fragment token and sends it as a bearer header", async () => {
  const values = new Map();
  const session = {
    getItem: (key) => values.get(key) || null,
    setItem: (key, value) => values.set(key, value),
  };
  let options;
  const client = createApiClient({
    fetchImpl: async (_path, received) => {
      options = received;
      return response(200, { ok: true });
    },
    session,
    browserLocation: { hash: "#token=secret", pathname: "/", search: "" },
    browserHistory: { replaceState() {} },
  });

  assert.deepEqual(await client.get("/api/v1/status"), { ok: true });
  assert.equal(values.get("stoker-ui-token"), "secret");
  assert.equal(options.headers.Authorization, "Bearer secret");
  assert.equal(options.cache, "no-store");
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
    browserLocation: { hash: "", pathname: "/", search: "" },
    browserHistory: { replaceState() {} },
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
    browserLocation: { hash: "", pathname: "/", search: "" },
    browserHistory: { replaceState() {} },
  });
  await assert.rejects(client.get("/api"), /Request failed \(502\)/);
});
