import assert from "node:assert/strict";
import test from "node:test";

import { isInteractiveEditing } from "../../modules/controller.js";

const elements = (overrides = {}) => ({
  jobDialog: { open: false },
  confirmDialog: { open: false },
  ...overrides,
});

test("poll rendering pauses for focused fields, open forms, confirmations, and description edits", () => {
  globalThis.document = { activeElement: { tagName: "DIV" } };
  assert.equal(isInteractiveEditing(elements()), false);

  document.activeElement = { tagName: "INPUT" };
  assert.equal(isInteractiveEditing(elements()), true);
  document.activeElement = { tagName: "SELECT" };
  assert.equal(isInteractiveEditing(elements()), true);
  document.activeElement = { tagName: "TEXTAREA" };
  assert.equal(isInteractiveEditing(elements()), true);

  document.activeElement = { tagName: "BUTTON" };
  assert.equal(isInteractiveEditing(elements({ jobDialog: { open: true } })), true);
  assert.equal(isInteractiveEditing(elements({ confirmDialog: { open: true } })), true);
  assert.equal(isInteractiveEditing(elements(), true), true);
});
