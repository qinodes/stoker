import assert from "node:assert/strict";
import test from "node:test";

import {
  DIRECTORY_CACHE_LIMIT,
  applyLoad,
  beginLoad,
  cacheDirectory,
  cachedDirectory,
  createState,
  failLoad,
  pageInfo,
} from "../../modules/state.js";

const payload = (name) => ({
  config: { version: name },
  status: { timezone: { name: "UTC" } },
  jobs: { jobs: [{ id: name }], timezone: { name: "UTC" } },
  queue: { jobs: [], locked: false },
  settings: { config: {}, snapshots: [] },
});

test("stale load success and failure cannot overwrite newer state", () => {
  const state = createState();
  const first = beginLoad(state);
  const second = beginLoad(state);
  assert.equal(applyLoad(state, second, payload("new")), true);
  assert.equal(applyLoad(state, first, payload("old")), false);
  assert.equal(failLoad(state, first, new Error("stale")), false);
  assert.equal(state.config.version, "new");
  assert.equal(state.error, null);
});

test("pagination clamps empty, boundary, and final pages", () => {
  assert.deepEqual(pageInfo([], 9, 6).items, []);
  assert.equal(pageInfo([1, 2, 3], 0, 2).page, 1);
  assert.deepEqual(pageInfo([1, 2, 3], 9, 2).items, [3]);
});

test("directory cache expires and evicts least recently used entries", () => {
  const state = createState();
  cacheDirectory(state, "/fresh", { path: "/fresh" }, 100);
  assert.equal(cachedDirectory(state, "/fresh", 101).path, "/fresh");
  assert.equal(cachedDirectory(state, "/fresh", 30_000), null);
  for (let index = 0; index <= DIRECTORY_CACHE_LIMIT; index += 1) {
    cacheDirectory(state, `/${index}`, { index }, 100);
  }
  assert.equal(state.directoryCache.size, DIRECTORY_CACHE_LIMIT);
  assert.equal(state.directoryCache.has("/0"), false);
});
