import assert from "node:assert/strict";
import test from "node:test";

import { formatPolicyValue, policyInputIsValid, policyRouteKey } from "../../src/policy.ts";

test("policy formatter keeps disabled runtime distinct from zero", () => {
  assert.equal(formatPolicyValue(null, "milliseconds"), "Disabled");
  assert.equal(formatPolicyValue(500, "milliseconds"), "500 milliseconds");
  assert.equal(formatPolicyValue(64, "MB"), "64 MB");
});

test("policy input validation accepts positive safe integers only", () => {
  assert.equal(policyInputIsValid("1"), true);
  assert.equal(policyInputIsValid("0"), false);
  assert.equal(policyInputIsValid("0", true), true);
  assert.equal(policyInputIsValid(""), false);
  assert.equal(policyInputIsValid("1.5"), false);
  assert.equal(policyInputIsValid(String(Number.MAX_SAFE_INTEGER + 1)), false);
});

test("policy route keys follow the CLI spelling", () => {
  assert.equal(policyRouteKey("max_bytes_per_job"), "max-bytes-per-job");
});
