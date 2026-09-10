import assert from "node:assert/strict";
import test from "node:test";

import {
  classForState,
  formatDate,
  isTerminalState,
  limitUnicode,
  routeTitle,
  shortId,
} from "../../src/formatters.ts";
import type { Route } from "../../src/types.ts";

test("formatters preserve Unicode boundaries and state classes", () => {
  assert.equal(limitUnicode("A🔥B", 2), "A🔥");
  assert.equal(shortId("1234567890"), "12345678…");
  assert.equal(classForState("RUNNING"), "state-running");
  assert.equal(classForState(null), "state-");
});

test("date formatting handles missing, invalid, and invalid timezone input", () => {
  assert.equal(formatDate(null), "—");
  assert.equal(formatDate("not-a-date"), "—");
  assert.notEqual(formatDate("2026-01-01T00:00:00Z", "Not/AZone", "en-US"), "—");
});

test("route and terminal state vocabulary is stable", () => {
  assert.equal(routeTitle("configuration"), "Configuration");
  assert.equal(routeTitle("unknown" as Route), "Overview");
  assert.equal(isTerminalState("LOST"), true);
  assert.equal(isTerminalState("RUNNING"), false);
});
