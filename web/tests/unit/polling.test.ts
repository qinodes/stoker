import assert from "node:assert/strict";
import test from "node:test";

import { createRequestSequence } from "../../src/state.ts";

test("polling sequence rejects stale responses and accepts the newest request", () => {
  const sequence = createRequestSequence();
  const first = sequence.next();
  const second = sequence.next();
  assert.equal(sequence.isCurrent(first), false);
  assert.equal(sequence.isCurrent(second), true);
});
