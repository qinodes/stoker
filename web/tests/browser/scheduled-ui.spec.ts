import { expect, test } from "@playwright/test";
import { mockBackend } from "./support/mock-backend.ts";

test("scheduled navigation excludes serial pages", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await expect(page.locator('[data-route="workloads"]')).toBeVisible();
  await expect(page.locator('[data-route="jobs"]')).toHaveCount(0);
  await expect(page.locator('[data-route="queue"]')).toHaveCount(0);
});

test("workspace queue lock is operable from the shared topbar", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");

  const control = page.locator('[data-action="workspace-queue-lock"]');
  await expect(control).toHaveText("Lock queue");
  await expect(control).toHaveClass(/unlocked/);
  await expect(control.locator(".status-dot")).toBeVisible();
  await expect(page.locator(".workspace-lock-state")).toHaveCount(0);
  await expect(page.locator(".topbar-actions > *").last()).toHaveClass(/language-control/);
  await expect(page.getByRole("button", { name: /refresh/i })).toHaveCount(0);
  await control.click();
  await expect(control).toHaveText("Unlock queue");
  await expect(control).toHaveClass(/locked/);
  await expect(page.locator('[data-queue-lock-state="locked"]')).toBeVisible();
  await control.click();
  await page.locator("#confirm-accept").click();
  await expect(control).toHaveText("Lock queue");
});

test("Sources keeps the shared queue lock and custom source file picker", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.addInitScript(() => {
    Object.defineProperty(window, "showSaveFilePicker", {
      configurable: true,
      value: async () => ({ createWritable: async () => ({ write: async () => {}, close: async () => {} }) }),
    });
  });
  await page.goto("/");

  await expect(page.locator('[data-action="workspace-queue-lock"]')).toBeVisible();
  await page.locator('[data-route="sources"]').click();
  await expect(page.locator('[data-source-queue]')).toHaveCount(0);
  await expect(page.locator(".source-mode-actions")).toHaveCount(0);
  await expect(page.locator(".source-import-panel")).toBeVisible();
  await expect(page.locator(".source-import-panel").getByRole("button", { name: "Export" })).toBeVisible();
  await expect(page.locator(".source-import-panel").getByRole("button", { name: "Snapshot" })).toBeVisible();
  await expect(page.locator('[data-source-file-picker]')).toBeVisible();
  await expect(page.locator('[data-source-file-input]')).toHaveCSS("display", "none");
  await expect(page.locator(".source-mode-switch-arrow")).toHaveText("↔");
  await expect(page.getByRole("button", { name: "Dry run" })).toBeDisabled();
  await page.getByRole("button", { name: "Export" }).click();
  await expect(page.locator("#toast-region")).toContainText("A second source copy was saved.");
  await expect(page.getByText("Document loaded; previous preview cleared.", { exact: true })).toHaveCount(0);
});

test("policy omits the redundant change protection row", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="policy"]').click();
  await expect(page.getByText("Change protection", { exact: true })).toHaveCount(0);
});

test("a scheduled draft Flow can be deleted after confirmation", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("row", { name: /Nightly Flow nightly-flow/ }).click();

  await page.getByRole("button", { name: "Delete Flow" }).click();
  await expect(page.getByRole("heading", { name: "Delete Flow?" })).toBeVisible();
  await page.locator("#confirm-accept").click();
  await expect(page.getByText("No Flows are available")).toBeVisible();
});

test("scheduled saves show feedback and configuration actions retain breathing room", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");

  await page.locator('[data-route="configuration"]').click();
  await expect(page.locator("#timezone-form .form-row")).toHaveCSS("margin-top", "12px");

  await page.locator('[data-route="policy"]').click();
  await page.getByRole("spinbutton", { name: "Maximum concurrency" }).fill("3");
  await page.locator(".policy-gate-action").getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.locator("#toast-region")).toContainText("Change saved to the server.");
});

test("scheduled status pill labels are optically centered", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  const label = page.locator(".state-badge-label").first();
  await expect(label).toHaveCSS("transform", "matrix(1, 0, 0, 1, 0, 1)");
});

test("scheduled state badges and run headers keep their content vertically centered", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");

  await page.locator('[data-route="workloads"]').click();
  await expect(page.locator(".state-badge").first()).toHaveCSS("line-height", "10px");

  await page.locator('[data-route="runs"]').click();
  const runHeader = page.locator(".data-table thead th").first();
  await expect(runHeader).toHaveCSS("padding-top", "12px");
  await expect(runHeader).toHaveCSS("padding-bottom", "12px");
  await expect(runHeader).toHaveCSS("vertical-align", "middle");
});

test("server mode change clears serial content until accepted", async ({ page }) => {
  const backend = await mockBackend(page, { workspaceMode: "serial" });
  await page.goto("/");
  // This establishes that the client has accepted the serial workspace before
  // the server changes. The heading expectation below then waits on the
  // client's real two-second mode poll instead of an arbitrary delay.
  await expect(page.getByRole("heading", { name: "See what’s running and what’s next." })).toBeVisible();
  await page.locator('[data-route="jobs"]').click();
  await expect(page.locator(".jobs-table")).toBeVisible();
  backend.workspaceMode = "scheduled";
  await expect(page.getByRole("heading", { name: /server mode changed/i })).toBeVisible();
  await expect(page.locator(".jobs-table")).toHaveCount(0);
  await page.getByRole("button", { name: "Later" }).click();
  await expect(page.getByRole("heading", { name: /server mode changed/i })).toBeVisible();
  await page.getByRole("button", { name: "Switch now" }).click();
  await expect(page.locator('[data-route="workloads"]')).toBeVisible();
});

test("scheduled workloads create a Flow, add a task, commit, run, and open its attempt log", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await expect(page.locator('[data-route="workloads"]')).toBeVisible();
  await page.locator('[data-route="workloads"]').click();
  await page.locator('[data-action="new-flow"]').click();
  await page.locator("#flow-id").fill("release-flow");
  await page.locator("#flow-name").fill("Release Flow");
  await page.locator("#flow-owner").fill("alice");
  await page.locator("#new-flow-form").getByRole("button", { name: "Create Flow" }).click();
  await page.getByRole("row", { name: /Release Flow release-flow/ }).click();
  await page.locator("#flow-task-id").fill("build");
  await page.locator(".task-editor input").nth(1).fill("Build");
  await page.locator(".task-editor input").nth(2).fill("echo build");
  await page.locator(".task-editor input").nth(3).fill("/workspace");
  await page.locator(".task-editor").getByRole("button", { name: "Add task" }).click();
  await page.getByRole("button", { name: "Commit", exact: true }).click();
  await page.getByRole("button", { name: "Run now", exact: true }).click();
  await page.locator('[data-route="runs"]').click();
  await page.getByText("release-flow", { exact: true }).click();
  await expect(page.getByRole("heading", { name: "Run detail" })).toBeVisible();
  await page.locator('[data-route="logs"]').click();
  await page.locator("#scheduled-log-run").selectOption("50000000-0000-4000-8000-000000000005");
  await page.locator("#scheduled-log-task").selectOption("build");
  await page.locator("#scheduled-log-attempt").selectOption("1");
  await expect(page.locator(".log-output")).toContainText("attempt output");
});

test("a frozen Flow keeps its typed schedule after a revision conflict", async ({ page }) => {
  const backend = await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("row", { name: /Nightly Flow nightly-flow/ }).click();
  await page.locator("#flow-task-id").fill("build");
  await page.locator(".task-editor input").nth(1).fill("Build");
  await page.locator(".task-editor input").nth(2).fill("echo build");
  await page.locator(".task-editor input").nth(3).fill("/workspace");
  await page.locator(".task-editor").getByRole("button", { name: "Add task" }).click();
  await page.getByRole("button", { name: "Commit", exact: true }).click();
  await page.getByRole("button", { name: "Freeze for edits" }).click();
  await page.getByRole("button", { name: "Edit schedule" }).click();
  const schedule = page.locator(".schedule-inputs input");
  await schedule.fill("2031-01-01T00:00:00Z");
  backend.revisionConflict = true;
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("This Flow changed on the server. Reload it before applying your draft.")).toBeVisible();
  await expect(schedule).toHaveValue("2031-01-01T00:00:00Z");
});

test("scheduled standalone jobs are available only in the Workloads job tab", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("tab", { name: "Standalone jobs" }).click();
  await expect(page.getByText("Standalone scheduled job", { exact: true })).toBeVisible();
  await expect(page.locator('[data-route="jobs"]')).toHaveCount(0);
});

test("sync mode disables Flow edits, previews removal, and syncs only after confirmation", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="sources"]').click();
  await page.getByRole("button", { name: "Sync", exact: true }).click();
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("row", { name: /Nightly Flow nightly-flow/ }).click();
  await expect(page.getByText("This Flow is managed by sync source mode. Direct edits are unavailable.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Add task", exact: true })).toHaveCount(0);
  await page.locator('[data-route="sources"]').click();
  await page.locator('input[type="file"]').setInputFiles({ name: "source.json", mimeType: "application/json", buffer: Buffer.from('{"version":1,"flows":[]}') });
  const dryRun = page.getByRole("button", { name: "Dry run" });
  await expect(dryRun).toBeEnabled();
  await expect(dryRun).toHaveClass(/source-action-ready/);
  await dryRun.click();
  await expect(dryRun).not.toHaveClass(/source-action-ready/);
  await expect(page.getByRole("heading", { name: "Sync preview" })).toBeVisible();
  await expect(page.locator(".source-preview-panel")).toHaveCSS("margin-top", "18px");
  await expect(page.getByText("Removed").locator("..")) .toContainText("1");
  const syncRequest = page.waitForRequest((request) => request.method() === "POST" && new URL(request.url()).pathname === "/api/v1/scheduled/sources/sync");
  await page.getByRole("button", { name: "Confirm sync" }).click();
  await syncRequest;
  await expect(page.getByRole("button", { name: "Dry run" })).toBeDisabled();
  await expect(page.getByText("Document loaded; previous preview cleared.", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Sync preview" })).toHaveCount(0);
  await page.locator('[data-route="workloads"]').click();
  await expect(page.getByText("No Flows are available")).toBeVisible();
});

test("zero and full capacity plus recovery fencing explain unavailable actions", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled", maxConcurrency: 0, activeAttempts: 0, recoveryFence: true });
  await page.goto("/");
  await expect(page.locator(".scheduled-metrics")).toContainText("0/0");
  await page.locator('[data-route="policy"]').click();
  await expect(page.getByText("Concurrency changes are unavailable while the server reports an unsafe state.")).toBeVisible();
  await expect(page.getByRole("spinbutton", { name: "Maximum concurrency" })).toBeDisabled();
  await page.locator('[data-route="runs"]').click();
  await expect(page.getByText("Recovery reconciliation required")).toBeVisible();
  await expect(page.getByRole("button", { name: "Reconcile recovery" })).toBeDisabled();
});

test("scheduled-to-serial transition clears scheduled data and holds after deferral", async ({ page }) => {
  const backend = await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await expect(page.locator('[data-route="workloads"]')).toBeVisible();
  backend.workspaceMode = "serial";
  await expect(page.getByRole("heading", { name: /server mode changed to serial/i })).toBeVisible();
  await expect(page.locator('[data-route="workloads"]')).toHaveCount(0);
  await page.getByRole("button", { name: "Later" }).click();
  await expect(page.getByRole("heading", { name: /server mode changed to serial/i })).toBeVisible();
});

test("scheduled locales fit a 320px viewport without horizontal document overflow", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto("/");
  await expect(page.locator('[data-route="workloads"]')).toBeVisible();
  for (const locale of ["en", "ja", "zh-TW"] as const) {
    await page.locator(".language-picker").click();
    await page.locator(`[data-locale="${locale}"]`).click();
    await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  }
});
