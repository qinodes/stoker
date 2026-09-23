import { expect, test } from "@playwright/test";
import { mockBackend } from "./support/mock-backend.ts";

test("scheduled pages align their content and show the route only in the topbar", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");

  await page.locator('[data-route="workloads"]').click();
  const reference = await page.locator(".page").boundingBox();
  const heading = await page.locator(".page-heading h1").boundingBox();
  expect(reference).not.toBeNull();
  expect(heading).not.toBeNull();

  for (const route of ["overview", "workloads", "runs", "logs", "sources", "configuration", "policy"]) {
    await page.locator(`[data-route="${route}"]`).click();
    await expect(page.locator(".page")).toHaveAttribute("data-view", route);
    await expect(page.locator(".page-heading h1")).toBeVisible();
    const box = await page.locator(".page").boundingBox();
    const title = await page.locator(".page-heading h1").boundingBox();
    expect(box?.x).toBe(reference!.x);
    expect(box?.width).toBe(reference!.width);
    expect(title?.y).toBe(heading!.y);
    await expect(page.locator(".page-heading .eyebrow")).toHaveCount(0);
    await expect(page.locator("#breadcrumb-current")).toBeVisible();
  }
});

test("scheduled content keeps its left edge when a page needs scrolling", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.setViewportSize({ width: 1920, height: 600 });
  await page.goto("/");
  await expect(page.locator(".main-content")).toHaveCSS("scrollbar-gutter", "stable");
  await page.locator('[data-route="workloads"]').click();
  await expect(page.locator(".page")).toHaveAttribute("data-view", "workloads");
  const reference = await page.locator(".page-heading").boundingBox();
  expect(reference).not.toBeNull();
  for (const route of ["configuration", "policy"]) {
    await page.locator(`[data-route="${route}"]`).click();
    await expect(page.locator(".page")).toHaveAttribute("data-view", route);
    const heading = await page.locator(".page-heading").boundingBox();
    expect(heading?.x).toBe(reference!.x);
    expect(heading?.width).toBe(reference!.width);
  }
});

test("scheduled navigation excludes serial pages", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await expect(page.locator('[data-route="workloads"]')).toBeVisible();
  await expect(page.locator('[data-route="jobs"]')).toHaveCount(0);
  await expect(page.locator('[data-route="queue"]')).toHaveCount(0);
});

test("scheduled overview separates summary metrics from run details", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");

  await expect(page.getByText("Next occurrences", { exact: true })).toHaveCount(0);
  await expect(page.locator(".scheduled-metrics .metric-card")).toHaveCount(2);
  await expect(page.locator(".scheduled-metrics")).toContainText("Flows");
  await expect(page.locator(".scheduled-metrics")).toContainText("0 live · 1 draft");
  await expect(page.locator(".scheduled-overview-grid > .panel")).toHaveCount(3);
  await expect(page.locator(".scheduled-overview-grid")).toContainText("Active runs");
  await expect(page.locator(".scheduled-overview-grid")).toContainText("Recent failures");
  await expect(page.locator(".scheduled-overview-grid")).toContainText("No recovery required");
  await expect(page.locator('.scheduled-overview-grid .panel-link[href="#runs"]')).toHaveText([
    "View active runs →",
    "View failures →",
    "View recovery →",
  ]);
});

test("scheduled overview cards show a fixed-height preview as failures accumulate", async ({ page }) => {
  const recentFailures: Array<{ run_id: string; flow_id: string; state: string; finished_at: string }> = [];
  await mockBackend(page, { workspaceMode: "scheduled", recentFailures });
  await page.goto("/");
  const cards = page.locator(".scheduled-overview-grid > .panel");
  await expect(cards).toHaveCount(3);
  const emptyHeight = (await cards.first().boundingBox())!.height;

  recentFailures.push({ run_id: "failed-run", flow_id: "nightly-flow", state: "FAILED", finished_at: "2026-09-23T10:00:00Z" });
  await page.reload();
  await expect(cards.nth(1)).toContainText("nightly-flow");
  const firstRowGap = await cards.nth(1).evaluate((card) =>
    card.querySelector(".occurrence-row")!.getBoundingClientRect().top - card.querySelector(".panel-header")!.getBoundingClientRect().bottom,
  );
  expect(firstRowGap).toBeLessThanOrEqual(8);
  const heights = await cards.evaluateAll((elements) => elements.map((element) => element.getBoundingClientRect().height));
  expect(new Set(heights).size).toBe(1);
  expect(heights[0]).toBeGreaterThanOrEqual(emptyHeight);
  expect(heights[0]).toBeLessThanOrEqual(280);

  for (let index = 2; index <= 6; index += 1) {
    recentFailures.push({ run_id: `failed-run-${index}`, flow_id: `flow-${index}`, state: "FAILED", finished_at: "2026-09-23T10:00:00Z" });
  }
  await page.reload();
  await expect(cards.nth(1).locator(".occurrence-row")).toHaveCount(3);
  await expect(cards.nth(1).locator(".overview-panel-count")).toHaveText("6");
  await expect(cards.nth(1).locator('a[href="#runs"]')).toBeVisible();
  const crowdedHeights = await cards.evaluateAll((elements) => elements.map((element) => element.getBoundingClientRect().height));
  expect(crowdedHeights).toEqual(heights);
  const failureList = cards.nth(1).locator(".occurrence-list");
  const listScroll = await failureList.evaluate((element) => ({ content: element.scrollHeight, visible: element.clientHeight }));
  expect(listScroll.content).toBeLessThanOrEqual(listScroll.visible);

  await page.setViewportSize({ width: 320, height: 700 });
  const mobileBounds = await cards.nth(1).evaluate((card) => ({
    cardBottom: card.getBoundingClientRect().bottom,
    lastRowBottom: card.querySelector(".occurrence-row:last-child")!.getBoundingClientRect().bottom,
  }));
  expect(mobileBounds.lastRowBottom).toBeLessThan(mobileBounds.cardBottom);
});

test("scheduled overview highlights an active recovery fence", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled", recoveryFence: true });
  await page.goto("/");

  const recovery = page.locator(".overview-run-panel.recovery-warning");
  await expect(recovery).toBeVisible();
  await expect(recovery).toContainText("Recovery fence is active");
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

test("workspace mode switch changes modes only after the queue is locked", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "serial" });
  await page.goto("/");

  const modeSwitch = page.locator(".sidebar-mode-control [data-workspace-mode-switch]");
  await expect(modeSwitch).toHaveAttribute("data-workspace-mode", "serial");
  await expect(modeSwitch.locator('[data-workspace-mode-option="serial"]')).toHaveAttribute("aria-checked", "true");
  await modeSwitch.locator('[data-workspace-mode-option="scheduled"]').click();
  await expect(page.locator("#toast-region")).toContainText("queue is unlocked; run 'stoker queue lock' first");
  await page.locator('[data-action="workspace-queue-lock"]').click();
  await expect(page.locator('[data-action="workspace-queue-lock"]')).toHaveClass(/locked/);
  await modeSwitch.locator('[data-workspace-mode-option="scheduled"]').click();
  await expect(page.locator('[data-route="workloads"]')).toBeVisible();
  await expect(modeSwitch).toHaveAttribute("data-workspace-mode", "scheduled");
  await expect(modeSwitch.locator('[data-workspace-mode-option="scheduled"]')).toHaveAttribute("aria-checked", "true");
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

test("scheduled Flows show five rows per page without desktop outer scrolling", async ({ page }) => {
  await page.setViewportSize({ width: 2187, height: 1173 });
  await mockBackend(page, { workspaceMode: "scheduled", flowCount: 11 });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();

  await expect(page.locator(".workloads-table tbody tr")).toHaveCount(5);
  const pagination = page.getByRole("navigation", { name: "Flows pagination" });
  await expect(pagination).toContainText("Showing 1–5 of 11");
  await expect.poll(() => page.locator(".main-content").evaluate((element) => element.scrollHeight === element.clientHeight)).toBe(true);
  await pagination.getByRole("button", { name: "Next" }).click();
  await expect(pagination).toContainText("Showing 6–10 of 11");
  await expect(page.getByText("flow-6", { exact: true })).toBeVisible();
});

test("scheduled Flow lists show frozen status alongside the primary status", async ({ page }) => {
  await mockBackend(page, {
    workspaceMode: "scheduled",
    flow: { committed: true, enabled: true, frozen: true },
  });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();

  const row = page.locator(".workloads-table tbody tr").first();
  await expect(row.locator(".state-badge")).toHaveCount(2);
  await expect(row.locator(".state-badge").nth(0)).toHaveText("RUNNING");
  await expect(row.locator(".state-badge").nth(1)).toHaveText("FROZEN");
});

test("Flow editing moves between clean, dirty, discarded, and applied states", async ({ page }) => {
  await mockBackend(page, {
    workspaceMode: "scheduled",
    flow: {
      committed: true,
      enabled: true,
      frozen: false,
      draft_revision: 7,
      has_draft: false,
      tasks: [{ task_id: "prepare", name: "Prepare", cwd: "/workspace", command: "echo prepare", retry: 0, dependencies: [], depend_mode: "all", sequence: 0 }],
    },
  });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("row", { name: /Nightly Flow nightly-flow/ }).click();

  await page.getByRole("button", { name: "Freeze for edits" }).click();
  await expect(page.getByRole("button", { name: "Exit edit mode" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Apply changes" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Discard draft" })).toHaveCount(0);

  await page.getByRole("button", { name: "Edit schedule" }).click();
  const scheduleDate = page.locator("#schedule-date");
  const scheduleHour = page.getByRole("combobox", { name: "Hour" });
  const scheduleMinute = page.getByRole("combobox", { name: "Minute" });
  await expect(scheduleDate).toHaveValue("09/22/2026");
  await expect(scheduleHour).toHaveValue("00");
  await expect(scheduleMinute).toHaveValue("00");
  await scheduleDate.fill("2026-09-23");
  await scheduleHour.selectOption("01");
  await scheduleMinute.selectOption("30");
  const scheduleRequest = page.waitForRequest((request) => request.method() === "PUT" && new URL(request.url()).pathname.endsWith("/schedule"));
  await page.getByRole("button", { name: "Save", exact: true }).click();
  expect((await scheduleRequest).postDataJSON().schedule).toEqual({ kind: "once", at: "2026-09-23T01:30:00Z" });
  await expect(page.getByRole("button", { name: "Apply changes" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Discard draft" })).toBeVisible();

  await page.getByRole("button", { name: "Discard draft" }).click();
  await expect(page.getByRole("button", { name: "Exit edit mode" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Apply changes" })).toHaveCount(0);
  await expect(scheduleDate).toHaveValue("09/22/2026");
  await expect(scheduleHour).toHaveValue("00");
  await expect(scheduleMinute).toHaveValue("00");

  await scheduleDate.fill("2026-09-24");
  await scheduleHour.selectOption("02");
  await scheduleMinute.selectOption("30");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page.getByRole("button", { name: "Apply changes" }).click();
  await expect(page.getByRole("button", { name: "Freeze for edits" })).toBeVisible();

  await page.getByRole("button", { name: "Freeze for edits" }).click();
  await page.getByRole("combobox", { name: "Schedule type" }).selectOption("daily");
  await page.locator("#schedule-timezone").fill("Tokyo");
  await page.locator("#schedule-timezone").press("ArrowDown");
  await page.locator("#schedule-timezone").press("Enter");
  await expect(page.locator("#schedule-timezone")).toHaveValue("Asia/Tokyo");
  await page.getByRole("button", { name: "Exit edit mode" }).click();
  await expect(page.getByRole("button", { name: "Freeze for edits" })).toBeVisible();
});

test("a frozen Flow edits a task in the styled dialog", async ({ page }) => {
  await mockBackend(page, {
    workspaceMode: "scheduled",
    flow: {
      committed: true,
      enabled: true,
      frozen: true,
      draft_revision: 7,
      has_draft: false,
      tasks: [{ task_id: "prepare", name: "Prepare", cwd: "/workspace", command: "echo prepare", retry: 0, dependencies: [], depend_mode: "all", sequence: 0 }],
    },
  });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("row", { name: /Nightly Flow nightly-flow/ }).click();

  await page.locator(".task-card").getByRole("button", { name: "Edit" }).click();
  const dialog = page.getByRole("dialog", { name: "Edit task command" });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator("textarea")).toHaveValue("echo prepare");
  await dialog.locator("textarea").fill("echo changed");
  await dialog.getByRole("button", { name: "Save" }).click();

  await expect(dialog).toHaveCount(0);
  await expect(page.locator(".task-card")).toContainText("echo changed");
  await expect(page.getByRole("button", { name: "Apply changes" })).toBeVisible();
  await expect(page.locator(".task-card-actions")).toHaveCSS("gap", "8px");
  await expect(page.locator(".flow-meta .state-badge")).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(page.locator(".flow-meta .state-badge")).toHaveCSS("color", "rgb(117, 214, 163)");
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
  await expect(page.locator(".sidebar-mode-control [data-workspace-mode-switch]")).toHaveAttribute("data-workspace-mode", "scheduled");
  await expect(page.locator(".sidebar-mode-control [data-workspace-mode-switch]")).toHaveAttribute("aria-busy", "true");
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
  const scheduleKind = page.locator("#new-flow-form .schedule-kind-field select");
  await expect(scheduleKind).toHaveValue("once");
  await expect(scheduleKind.locator("option")).toHaveCount(3);
  await expect(page.locator("#new-flow-form .schedule-inputs")).toHaveCSS("margin-top", "18px");
  await scheduleKind.selectOption("daily");
  const created = page.waitForRequest((request) => request.url().endsWith("/api/v1/scheduled/flows") && request.method() === "POST");
  await page.locator("#new-flow-form").getByRole("button", { name: "Create Flow" }).click();
  expect((await created).postDataJSON()).toMatchObject({ schedule: { kind: "daily", time: "00:00", timezone: "UTC" } });
  await page.getByRole("row", { name: /Release Flow release-flow/ }).click();
  await page.locator('[data-action="browse-scheduled-directory"]').click();
  await expect(page.locator("#directory-browser-dialog")).toBeVisible();
  await expect(page.locator('[data-directory="/workspace/child"]')).toBeVisible();
  await page.locator('[data-directory="/workspace/child"]').click();
  await page.locator("#directory-browser-dialog [data-action=\"choose-directory\"]").click();
  await expect(page.locator("#flow-task-cwd")).toHaveValue("/workspace/child");
  await expect(page.locator("#directory-browser-dialog")).not.toBeVisible();
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
  await page.locator(".language-picker").click();
  await page.locator('[data-locale="zh-TW"]').click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-TW");
  await page.locator("#scheduled-log-run").selectOption("50000000-0000-4000-8000-000000000005");
  await page.locator("#scheduled-log-task").selectOption("build");
  await page.locator("#scheduled-log-attempt").selectOption("1");
  await expect(page.locator('#scheduled-log-task option[value="build"]')).toHaveText("build · 成功");
  await expect(page.locator(".log-output")).toContainText("attempt output");
});

test("required Flow fields use the selected UI language", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.locator('[data-action="new-flow"]').click();
  const expected = { en: "Please fill out this field.", "zh-TW": "請填寫此欄位。", ja: "この項目を入力してください。" } as const;
  for (const locale of ["en", "zh-TW", "ja"] as const) {
    await page.locator(".language-picker").click();
    await page.locator(`[data-locale="${locale}"]`).click();
    if (locale !== "en") await expect.poll(() => page.locator("#flow-id").evaluate((input: HTMLInputElement) => input.validationMessage)).toBe(expected[locale]);
    await page.locator('#new-flow-form button[type="submit"]').click();
    await expect.poll(() => page.locator("#flow-id").evaluate((input: HTMLInputElement) => input.validationMessage)).toBe(expected[locale]);
  }
  await page.locator("#flow-id").fill("localized-flow");
  await page.locator("#flow-name").fill("Localized Flow");
  await page.locator("#flow-owner").fill("alice");
  await page.locator('#new-flow-form button[type="submit"]').click();
  await expect.poll(() => page.locator("#new-flow-schedule-date").evaluate((input: HTMLInputElement) => input.validationMessage)).toBe(expected.ja);
  await page.locator("#new-flow-schedule-date").fill("2026/09/23");
  await page.locator('#new-flow-form button[type="submit"]').click();
  await expect.poll(() => page.locator('#new-flow-schedule-time select').first().evaluate((input: HTMLSelectElement) => input.validationMessage)).toBe("リストから項目を選択してください。");
});

test("schedule errors update when the UI language changes", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.locator('[data-action="new-flow"]').click();
  await page.locator("#flow-id").fill("bad-zone");
  await page.locator("#flow-name").fill("Bad Zone");
  await page.locator("#flow-owner").fill("alice");
  await page.locator("#new-flow-form .schedule-kind-field select").selectOption("daily");
  await page.locator("#new-flow-schedule-timezone").fill("Invalid/Zone");
  await page.locator('#new-flow-form button[type="submit"]').click();
  await expect(page.locator("#new-flow-form .schedule-error")).toHaveText("Choose an IANA timezone from the suggestions.");
  await page.locator(".language-picker").click();
  await page.locator('[data-locale="zh-TW"]').click();
  await expect(page.locator("#new-flow-form .schedule-error")).toHaveText("請從建議項目選擇 IANA 時區。");
});

test("frozen Flow schedule validation follows the selected language", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled", flow: { committed: true, enabled: true, frozen: true } });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("row", { name: /Nightly Flow nightly-flow/ }).click();
  await page.getByRole("button", { name: "Edit schedule" }).click();
  await page.locator(".flow-schedule-editor .schedule-kind-field select").selectOption("periodic");
  await page.locator(".flow-schedule-editor .schedule-kind-field select").selectOption("once");
  await page.locator(".language-picker").click();
  await page.locator('[data-locale="ja"]').click();
  await page.locator('.flow-schedule-editor button[type="submit"]').click();
  await expect.poll(() => page.locator("#schedule-date").evaluate((input: HTMLInputElement) => input.validationMessage)).toBe("この項目を入力してください。");
});

test("task fields and numeric limits use localized validation", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("row", { name: /Nightly Flow nightly-flow/ }).click();
  await page.locator(".language-picker").click();
  await page.locator('[data-locale="zh-TW"]').click();
  await page.locator('.task-editor button[type="submit"]').click();
  await expect.poll(() => page.locator("#flow-task-id").evaluate((input: HTMLInputElement) => input.validationMessage)).toBe("請填寫此欄位。");
  await page.locator("#flow-task-id").fill("build");
  await page.locator(".task-editor input").nth(1).fill("Build");
  await page.locator(".task-editor input").nth(2).fill("echo build");
  await page.locator("#flow-task-cwd").fill("/workspace");
  await page.locator('.task-editor input[type="number"]').fill("-1");
  await page.locator('.task-editor button[type="submit"]').click();
  await expect.poll(() => page.locator('.task-editor input[type="number"]').evaluate((input: HTMLInputElement) => input.validationMessage)).toBe("請輸入有效的數字。");
});

test("new Flow opens at the bottom and creation positions Flow detail at the top", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.setViewportSize({ width: 1280, height: 600 });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.locator('[data-action="new-flow"]').click();
  await expect(page.locator("#new-flow-form")).toBeVisible();
  await expect.poll(() => page.locator(".main-content").evaluate((scroller) => scroller.scrollTop)).toBeGreaterThan(0);
  const initialScroll = await page.locator(".main-content").evaluate((scroller) => ({ current: scroller.scrollTop, max: scroller.scrollHeight - scroller.clientHeight }));
  expect(initialScroll.max - initialScroll.current).toBeLessThanOrEqual(2);

  await page.locator("#flow-id").fill("scroll-flow");
  await page.locator("#flow-name").fill("Scroll Flow");
  await page.locator("#flow-owner").fill("alice");
  await page.locator("#new-flow-form .schedule-kind-field select").selectOption("daily");
  await page.locator("#new-flow-form").getByRole("button", { name: "Create Flow" }).click();
  await expect(page.locator("#new-flow-form")).toHaveCount(0);
  await expect(page.locator(".task-editor")).toBeVisible();
  const position = await page.evaluate(() => {
    const scroller = document.querySelector(".main-content")!;
    const detail = document.querySelector(".flow-detail")!.getBoundingClientRect();
    const task = document.querySelector(".task-editor")!.getBoundingClientRect();
    const topbar = document.querySelector(".topbar")!.getBoundingClientRect();
    return { detailTop: detail.top, taskTop: task.top, topbarBottom: topbar.bottom, current: scroller.scrollTop, max: scroller.scrollHeight - scroller.clientHeight };
  });
  expect(position.detailTop).toBeGreaterThanOrEqual(position.topbarBottom);
  expect(position.detailTop).toBeLessThanOrEqual(position.topbarBottom + 40);
  expect(position.taskTop).toBeLessThan(600);
  expect(position.max - position.current).toBeGreaterThan(20);
});

test("new Flow scrolls the document on mobile", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.setViewportSize({ width: 375, height: 667 });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.locator('[data-action="new-flow"]').click();
  await expect(page.locator("#new-flow-form")).toBeVisible();
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeGreaterThan(0);
  const opened = await page.evaluate(() => ({ current: window.scrollY, max: document.documentElement.scrollHeight - window.innerHeight }));
  expect(opened.max - opened.current).toBeLessThanOrEqual(2);

  await page.locator("#flow-id").fill("mobile-flow");
  await page.locator("#flow-name").fill("Mobile Flow");
  await page.locator("#flow-owner").fill("alice");
  await page.locator("#new-flow-form .schedule-kind-field select").selectOption("daily");
  await page.locator("#new-flow-form").getByRole("button", { name: "Create Flow" }).click();
  await expect(page.locator(".task-editor")).toBeVisible();
  const created = await page.evaluate(() => ({ detailTop: document.querySelector(".flow-detail")!.getBoundingClientRect().top, current: window.scrollY, max: document.documentElement.scrollHeight - window.innerHeight }));
  expect(created.detailTop).toBeGreaterThanOrEqual(0);
  expect(created.detailTop).toBeLessThanOrEqual(40);
  expect(created.max - created.current).toBeGreaterThan(20);
});

test("opening an existing Flow scrolls to the bottom of its detail", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.setViewportSize({ width: 1280, height: 600 });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.getByRole("row", { name: /Nightly Flow nightly-flow/ }).click();
  await expect(page.locator("#selected-flow-detail")).toBeVisible();
  await expect.poll(() => page.locator(".main-content").evaluate((scroller) => scroller.scrollTop)).toBeGreaterThan(0);
  const scroll = await page.locator(".main-content").evaluate((scroller) => ({ current: scroller.scrollTop, max: scroller.scrollHeight - scroller.clientHeight }));
  expect(scroll.max - scroll.current).toBeLessThanOrEqual(2);
});

test("localized schedule controls keep date labels in the active language", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.locator('[data-action="new-flow"]').click();
  const date = page.locator("#new-flow-schedule-date");
  const expected = { en: "Choose date", "zh-TW": "選擇日期", ja: "日付を選択" } as const;
  const expectedFormat = { en: "MM/DD/YYYY", "zh-TW": "YYYY/MM/DD", ja: "YYYY/MM/DD" } as const;
  const expectedValue = { en: "09/23/2026", "zh-TW": "2026/09/23", ja: "2026/09/23" } as const;
  for (const locale of ["en", "zh-TW", "ja"] as const) {
    await page.locator(".language-picker").click();
    await page.locator(`[data-locale="${locale}"]`).click();
    await expect(page.locator("html")).toHaveAttribute("lang", locale);
    await expect(date).toHaveAttribute("lang", locale);
    await expect(date).toHaveAttribute("title", expected[locale]);
    await expect(date).toHaveAttribute("aria-label", expected[locale]);
    await expect(date).toHaveAttribute("placeholder", expectedFormat[locale]);
    await expect(date).toHaveValue("");
    await date.fill("2026-09-23");
    await date.blur();
    await expect(date).toHaveValue(expectedValue[locale]);
    await date.fill("");
    await date.blur();
  }
});

test("schedule date picker uses the active language instead of the browser locale", async ({ page }) => {
  await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator('[data-route="workloads"]').click();
  await page.locator('[data-action="new-flow"]').click();
  const date = page.locator("#new-flow-schedule-date");
  const expected = {
    en: { clear: "Clear", today: "Today", previous: "Previous month", next: "Next month", weekday: "Sun" },
    "zh-TW": { clear: "清除", today: "今天", previous: "上個月", next: "下個月", weekday: "週日" },
    ja: { clear: "クリア", today: "今日", previous: "前月", next: "次月", weekday: "日" },
  } as const;
  for (const locale of ["en", "zh-TW", "ja"] as const) {
    await page.locator(".language-picker").click();
    await page.locator(`[data-locale="${locale}"]`).click();
    await date.click();
    const picker = page.locator(".schedule-date-picker");
    await expect(picker).toBeVisible();
    await expect(picker.getByRole("button", { name: expected[locale].clear, exact: true })).toBeVisible();
    await expect(picker.getByRole("button", { name: expected[locale].today, exact: true })).toBeVisible();
    await expect(picker.getByRole("button", { name: expected[locale].previous, exact: true })).toBeVisible();
    await expect(picker.getByRole("button", { name: expected[locale].next, exact: true })).toBeVisible();
    await expect(picker.locator(".schedule-date-weekday").first()).toHaveText(expected[locale].weekday);
    await picker.locator(".schedule-date-day:not(.outside-month)").first().click();
    await expect(date).not.toHaveValue("");
    await expect(picker).toHaveCount(0);
    await date.click();
    await expect(page.locator(".schedule-date-picker")).toBeVisible();
    await picker.getByRole("button", { name: expected[locale].clear, exact: true }).click();
  }
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
  const schedule = page.locator("#schedule-date");
  await schedule.fill("2031-01-01");
  backend.revisionConflict = true;
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("This Flow changed on the server. Reload it before applying your draft.")).toBeVisible();
  await expect(schedule).toHaveValue("01/01/2031");
  backend.revisionConflict = false;
  await page.getByRole("button", { name: "Reload Flow" }).click();
  await expect(page.getByText("This Flow changed on the server. Reload it before applying your draft.")).toHaveCount(0);
  const close = page.getByRole("button", { name: "Close", exact: true });
  await expect(close.locator("..")).toHaveClass("flow-detail-close");
  await close.click();
  await expect(page.getByRole("heading", { name: "Nightly Flow" })).toHaveCount(0);
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

test("mode transition breadcrumb follows the selected language", async ({ page }) => {
  const backend = await mockBackend(page, { workspaceMode: "scheduled" });
  await page.goto("/");
  await page.locator(".language-picker").click();
  await page.locator('[data-locale="zh-TW"]').click();
  backend.workspaceMode = "serial";
  await expect(page.getByRole("heading", { name: /伺服器模式已變更為 serial/ })).toBeVisible();
  await expect(page.locator("#breadcrumb-current")).toHaveText("模式變更");
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
