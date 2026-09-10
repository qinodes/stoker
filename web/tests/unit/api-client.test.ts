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

function session(values = new Map<string, string>()): Storage {
  return {
    get length() { return values.size; },
    clear() { values.clear(); },
    getItem(key) { return values.get(key) ?? null; },
    key(index) { return [...values.keys()][index] ?? null; },
    removeItem(key) { values.delete(key); },
    setItem(key, value) { values.set(key, value); },
  };
}

function location(hash = ""): Location {
  return { hash, pathname: "/", search: "" } as unknown as Location;
}

function history(): History {
  return {
    length: 0,
    scrollRestoration: "auto",
    state: null,
    back() {},
    forward() {},
    go() {},
    pushState(_data: unknown, _unused: string, _url?: string | URL | null) {},
    replaceState(_data: unknown, _unused: string, _url?: string | URL | null) {},
  };
}

test("client stores fragment token and sends it as a bearer header", async () => {
  const values = new Map<string, string>();
  let received: RequestInit | undefined;
  const client = createApiClient({
    fetchImpl: async (_path, options) => {
      received = options;
      return response(200, { ok: true });
    },
    session: session(values),
    browserLocation: location("#token=secret"),
    browserHistory: history(),
  });

  assert.deepEqual(await client.get("/api/v1/status"), { ok: true });
  assert.equal(values.get("stoker-ui-token"), "secret");
  assert.ok(received);
  assert.equal(new Headers(received.headers).get("Authorization"), "Bearer secret");
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
    session: session(),
    browserLocation: location(),
    browserHistory: history(),
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
    fetchImpl: async () => new Response("not json", { status: 502 }),
    session: session(),
    browserLocation: location(),
    browserHistory: history(),
  });
  await assert.rejects(client.get("/api"), /Request failed \(502\)/);
});
