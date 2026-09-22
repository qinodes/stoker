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
import { instantToLocalDateTime, localDateTimeToRfc3339 } from "../../src/scheduled/timezone.ts";

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

test("loading or closing a Flow clears an earlier revision conflict", () => {
  const conflicted = reduceScheduled(createScheduledState(), { type: "revisionConflict", value: true });
  const loaded = reduceScheduled(conflicted, {
    type: "flowLoaded",
    flow: { flow_id: "new-flow", frozen: false, draft_revision: 0, tasks: [] } as unknown as ScheduledFlow,
  });

  assert.equal(loaded.revisionConflict, false);
  assert.equal(loaded.selectedFlow?.flow_id, "new-flow");
  const closed = reduceScheduled({ ...loaded, revisionConflict: true }, { type: "flowClosed" });
  assert.equal(closed.selectedFlow, null);
  assert.equal(closed.revisionConflict, false);
});

test("sync can only submit its previewed hash", () => {
  const state = withSyncPreview(createScheduledState(), {
    hash: "sha256:abc", changed: true,
    diff: { added: 1, updated: 0, removed: 0, unchanged: 0 },
  });
  assert.equal(canConfirmSync(state, "sha256:abc"), true);
  assert.equal(canConfirmSync(state, "sha256:def"), false);
});

test("clearing a confirmed source import removes its document and preview", () => {
  const loaded = reduceScheduled(createScheduledState(), {
    type: "sourceDocumentLoaded",
    document: { text: "{}", hash: "sha256:source", value: {} },
  });
  const reviewed = withSyncPreview(loaded, {
    hash: "sha256:reviewed", changed: true,
    diff: { added: 0, updated: 0, removed: 0, unchanged: 1 },
  });
  const cleared = reduceScheduled(reviewed, { type: "sourceImportCleared" });
  assert.equal(cleared.sourceDocument, null);
  assert.equal(cleared.syncPreview, null);
});

test("IANA local schedule times round-trip and reject DST gaps", () => {
  assert.deepEqual(instantToLocalDateTime("2026-10-01T01:30:00Z", "Asia/Taipei"), {
    date: "2026-10-01",
    time: "09:30",
  });
  assert.deepEqual(localDateTimeToRfc3339("2026-10-01", "09:30", "Asia/Taipei"), {
    ok: true,
    value: "2026-10-01T01:30:00Z",
  });
  assert.deepEqual(localDateTimeToRfc3339("2026-03-08", "02:30", "America/New_York"), {
    ok: false,
    reason: "nonexistent",
  });
  assert.deepEqual(localDateTimeToRfc3339("2026-11-01", "01:30", "America/New_York"), {
    ok: true,
    value: "2026-11-01T05:30:00Z",
  });
});
