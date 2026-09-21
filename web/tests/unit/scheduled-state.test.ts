import assert from "node:assert/strict";
import test from "node:test";

import type { ScheduledFlow } from "../../src/types.ts";
import {
  canConfirmSync,
  createScheduledState,
  flowMutationRequest,
  reduceScheduled,
  withSyncPreview,
} from "../../src/scheduled/state.ts";

test("Flow mutations use the revision returned by the server", () => {
  const state = reduceScheduled(createScheduledState(), {
    type: "flowLoaded",
    flow: { flow_id: "nightly", frozen: true, draft_revision: 2, tasks: [] } as unknown as ScheduledFlow,
  });

  assert.equal(
    flowMutationRequest(state.selectedFlow!, { task_id: "publish" }).expected_draft_revision,
    2,
  );
});

test("sync can only submit its previewed hash", () => {
  const state = withSyncPreview(createScheduledState(), {
    hash: "sha256:abc", changed: true,
    diff: { added: 1, updated: 0, removed: 0, unchanged: 0 },
  });
  assert.equal(canConfirmSync(state, "sha256:abc"), true);
  assert.equal(canConfirmSync(state, "sha256:def"), false);
});
