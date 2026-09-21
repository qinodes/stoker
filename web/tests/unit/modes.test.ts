import assert from "node:assert/strict";
import test from "node:test";

import { ApiError } from "../../src/api.ts";
import { beginModeTransition, deferModeTransition, isRouteAllowed, modeFromApiError } from "../../src/modes.ts";
import { createState } from "../../src/state.ts";
import type { Job } from "../../src/types.ts";

test("mode change clears old data and invalidates the old route", () => {
  const state = createState("jobs");
  state.jobs = [{ id: "serial-job" } as Job];

  const next = beginModeTransition(state, "scheduled");

  assert.equal(next.modeTransition?.actualMode, "scheduled");
  assert.deepEqual(next.jobs, []);
  assert.equal(next.route, "mode-change");
  assert.equal(isRouteAllowed("scheduled", "queue"), false);
});

test("mode_changed is converted into a scheduled transition", () => {
  const error = new ApiError({ status: 409, code: "mode_changed", details: { mode: "scheduled" } });

  assert.equal(modeFromApiError(error), "scheduled");
});

test("deferred transition has no serial or scheduled records", () => {
  const state = deferModeTransition(beginModeTransition(createState(), "scheduled"));
  assert.equal(state.route, "mode-change");
  assert.deepEqual(state.jobs, []);
  assert.deepEqual(state.scheduled.flows, []);
});
