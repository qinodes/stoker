import assert from "node:assert/strict";
import test from "node:test";

import {
  DIRECTORY_CACHE_LIMIT,
  JOBS_PAGE_SIZE,
  cacheDirectory,
  cachedDirectory,
  createState,
  pageInfo,
} from "../../src/state.ts";

test("state starts with the overview route and empty workspace values", () => {
  const state = createState();
  assert.equal(state.route, "overview");
  assert.equal(state.loaded, false);
  assert.deepEqual(state.jobs, []);
  assert.equal(state.jobDraft.command, "");
});

test("pagination clamps empty, boundary, and final pages", () => {
  assert.equal(JOBS_PAGE_SIZE, 5);
  assert.deepEqual(pageInfo([], 9, 6).items, []);
  assert.equal(pageInfo([1, 2, 3], 0, 2).page, 1);
  assert.deepEqual(pageInfo([1, 2, 3], 9, 2).items, [3]);
});

test("directory cache expires and evicts the least recently used entries", () => {
  const cache = new Map();
  cacheDirectory(cache, "/fresh", { path: "/fresh", directories: [] }, 100);
  assert.equal(cachedDirectory(cache, "/fresh", 101)?.path, "/fresh");
  assert.equal(cachedDirectory(cache, "/fresh", 30_000), null);
  for (let index = 0; index <= DIRECTORY_CACHE_LIMIT; index += 1) {
    cacheDirectory(cache, `/${index}`, { path: `/${index}`, directories: [] }, 100);
  }
  assert.equal(cache.size, DIRECTORY_CACHE_LIMIT);
  assert.equal(cache.has("/0"), false);
});
