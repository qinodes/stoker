import assert from "node:assert/strict";
import test from "node:test";

import {
  escapeHtml,
  formatDate,
  isTerminalState,
  limitUnicode,
  routeTitle,
  shortId,
} from "../../modules/formatters.js";

test("formatters escape untrusted content and preserve Unicode boundaries", () => {
  assert.equal(escapeHtml('<script a="b">&'), "&lt;script a=&quot;b&quot;&gt;&amp;");
  assert.equal(limitUnicode("A🔥B", 2), "A🔥");
  assert.equal(shortId("1234567890"), "12345678…");
});

test("date formatting handles missing, invalid, and invalid timezone input", () => {
  assert.equal(formatDate(null), "—");
  assert.equal(formatDate("not-a-date"), "—");
  assert.notEqual(formatDate("2026-01-01T00:00:00Z", "Not/AZone", "en-US"), "—");
});

test("route and terminal state vocabulary is stable", () => {
  assert.equal(routeTitle("configuration"), "Configuration");
  assert.equal(routeTitle("unknown"), "Overview");
  assert.equal(isTerminalState("LOST"), true);
  assert.equal(isTerminalState("RUNNING"), false);
});
