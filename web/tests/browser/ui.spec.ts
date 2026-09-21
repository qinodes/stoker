import { expect, test, type Page, type Route as PlaywrightRoute, type Request } from "@playwright/test";
import type { Job, PolicyResponse, RootsResponse, SettingsResponse, Snapshot, StatusResponse, TimezoneInfo } from "../../src/types.ts";
import { LANGUAGE_STORAGE_KEY, translate, type Locale, type StaticKey } from "../../src/i18n/messages.ts";

interface MockModel {
  jobs: Job[];
  queue: Job[];
  locked: boolean;
  timezone: string;
  snapshots: Snapshot[];
  policy: PolicyResponse;
}

interface MockBackendOptions {
  onRequest?: (path: string, method: string) => void;
}

interface JobCreateRequest {
  name: string;
  user: string;
  cwd: string;
  command: string;
  description: string | null;
}

interface DescriptionRequest {
  description: string | null;
}

interface QueueMoveRequest {
  target_order: number;
}

interface TimezoneRequest {
  value: string;
}

const baseJob = (overrides: Partial<Job> = {}): Job => ({
  id: overrides.id || crypto.randomUUID(),
  name: "seed-job",
  user: "tester",
  cwd: "/workspace",
  command: ["echo", "hello"],
  command_line: "echo hello",
  state: "QUEUED",
  queue_order: 1,
  created_at: "2026-09-10T00:00:00Z",
  committed_at: "2026-09-10T00:01:00Z",
  started_at: null,
  finished_at: null,
  exit_code: null,
  pid: null,
  failure_detail: null,
  description: null,
  description_revision: 0,
  ...overrides,
});

async function mockBackend(page: Page, { onRequest = () => {} }: MockBackendOptions = {}): Promise<MockModel> {
  const seed = baseJob({ id: "10000000-0000-4000-8000-000000000001" });
  const model: MockModel = {
    jobs: [seed],
    queue: [seed],
    locked: false,
    timezone: "UTC",
    snapshots: [],
    policy: defaultPolicy(),
  };

  await page.route("**/api/v1/**", async (route: PlaywrightRoute) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    const method = request.method();
    onRequest(path, method);
    if (path === "/api/v1/ui/config") {
      return route.fulfill({ json: {
        version: "1.3.1-test",
        max_job_name_length: 128,
        max_job_user_length: 50,
        max_job_description_length: 200,
      } });
    }
    if (path === "/api/v1/workspace") {
      return route.fulfill({ json: {
        mode: "serial",
        queue_locked: model.locked,
        recovery_fence: false,
        scheduler: { running: false, pid: null, active_job: null, queued_jobs: model.queue.length },
        timezone: timezone(model),
        generated_at: "2026-09-10T00:00:00Z",
      } });
    }
    if (path === "/api/v1/status") return route.fulfill({ json: status(model) });
    if (path === "/api/v1/jobs" && method === "GET") {
      return route.fulfill({ json: { jobs: model.jobs, timezone: timezone(model) } });
    }
    if (path === "/api/v1/jobs" && method === "POST") {
      const body = requestJson<JobCreateRequest>(request);
      const job = baseJob({
        id: "20000000-0000-4000-8000-000000000002",
        name: body.name,
        user: body.user,
        cwd: body.cwd,
        command: body.command.split(" "),
        command_line: body.command,
        state: "DRAFT",
        queue_order: null,
        committed_at: null,
        description: body.description,
      });
      model.jobs.push(job);
      return route.fulfill({ status: 201, json: { job } });
    }
    if (path === "/api/v1/queue" && method === "GET") {
      return route.fulfill({ json: { jobs: model.queue, locked: model.locked } });
    }
    if (path === "/api/v1/policy" && method === "GET") return route.fulfill({ json: { ...model.policy, queue_locked: model.locked, can_update: model.locked, blocked_reason: model.locked ? null : "Lock the queue before changing policy.", active_jobs: [] } });
    const policyMatch = path.match(/^\/api\/v1\/policy\/([^/]+)$/);
    if (policyMatch && (method === "PUT" || method === "DELETE")) {
      if (!model.locked) return typedError(route, 409, "conflict", "Lock the queue before changing policy.");
      const key = policyMatch[1];
      const body = method === "PUT" ? requestJson<{ value: number }>(request) : null;
      if (!setMockPolicyValue(model.policy, key, body?.value)) {
        return typedError(route, 400, "invalid_input", "unknown policy key");
      }
      return route.fulfill({ json: { ...model.policy, queue_locked: true, can_update: true, blocked_reason: null, active_jobs: [] } });
    }
    if (path === "/api/v1/config" || path === "/api/v1/config/snapshots") {
      return route.fulfill({ json: configuration(model) });
    }
    if (path === "/api/v1/fs/roots") {
      return route.fulfill({ json: { default_path: "/workspace", locations: [{ kind: "root", label: "Workspace", path: "/workspace" }] } });
    }
    if (path === "/api/v1/fs/directories") {
      return route.fulfill({ json: { path: url.searchParams.get("path"), parent: "/", directories: [{ name: "child", path: "/workspace/child", is_symlink: false }], truncated: false, skipped_entries: 0 } });
    }
    const jobMatch = path.match(/^\/api\/v1\/jobs\/([^/]+)(?:\/(description|commit|cancel|logs))?$/);
    if (jobMatch) {
      const job = model.jobs.find((entry) => entry.id === jobMatch[1]);
      if (!job) return typedError(route, 404, "not_found", "job not found");
      const action = jobMatch[2];
      if (!action) return route.fulfill({ json: { job, working_directory_status: "planned", display_timezone: model.timezone } });
      if (action === "description") {
        const body = requestJson<DescriptionRequest>(request);
        job.description = body.description;
        job.description_revision = (job.description_revision ?? 0) + 1;
        return route.fulfill({ json: { job } });
      }
      if (action === "commit") {
        job.state = "QUEUED";
        job.queue_order = model.queue.length + 1;
        model.queue.push(job);
        return route.fulfill({ json: { job } });
      }
      if (action === "cancel") {
        job.state = "CANCELLED";
        return route.fulfill({ json: { job } });
      }
      return route.fulfill({ json: {
        job,
        stdout: "hello from stdout\n",
        stderr: "",
        stdout_available: true,
        stderr_available: false,
        stdout_truncated: false,
        stderr_truncated: false,
        message: null,
      } });
    }
    if (path === "/api/v1/queue/lock") {
      model.locked = true;
      return route.fulfill({ json: { jobs: model.queue, locked: true } });
    }
    if (path === "/api/v1/queue/unlock") {
      model.locked = false;
      return route.fulfill({ json: { jobs: model.queue, locked: false } });
    }
    const moveMatch = path.match(/^\/api\/v1\/queue\/([^/]+)\/move$/);
    if (moveMatch) {
      const index = model.queue.findIndex((job) => job.id === moveMatch[1]);
      const [job] = model.queue.splice(index, 1);
      model.queue.splice(requestJson<QueueMoveRequest>(request).target_order - 1, 0, job);
      model.queue.forEach((entry, order) => { entry.queue_order = order + 1; });
      return route.fulfill({ json: { jobs: model.queue, locked: model.locked } });
    }
    if (path === "/api/v1/config/timezone" && method === "PUT") {
      model.timezone = requestJson<TimezoneRequest>(request).value;
      return route.fulfill({ json: configuration(model) });
    }
    if (path === "/api/v1/config/timezone" && method === "DELETE") {
      model.timezone = "UTC";
      return route.fulfill({ json: configuration(model) });
    }
    if (path === "/api/v1/config/snapshot") {
      model.snapshots.unshift({ path: "/snapshots/config.json", valid: true, created_at: "2026-09-10T00:02:00Z", reason: "manual", timezone: model.timezone, error: null });
      return route.fulfill({ json: configuration(model) });
    }
    if (path === "/api/v1/config/restore") {
      model.timezone = "UTC";
      return route.fulfill({ json: configuration(model) });
    }
    if (path === "/api/v1/clean") return route.fulfill({ json: { removed: 0 } });
    return typedError(route, 404, "not_found", `unmocked ${method} ${path}`);
  });
  return model;
}

test("bundled browser creates and reads a job through the real Axum application stack", async ({ page, request }) => {
  const rootsResponse = await request.get("/api/v1/fs/roots");
  expect(rootsResponse.ok()).toBeTruthy();
  const roots = await rootsResponse.json() as RootsResponse;
  expect(roots.default_path).toBeTruthy();
  const name = `browser-real-${Date.now()}`;

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "See what’s running and what’s next." })).toBeVisible();
  await expect(page).toHaveTitle("Stoker");
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute("href", "/assets/logo-mark.png");
  const favicon = await request.get("/assets/logo-mark.png");
  expect(favicon.ok()).toBeTruthy();
  expect(favicon.headers()["content-type"]).toContain("image/png");
  await page.locator('[data-route="jobs"]').click();
  await page.locator('[data-action="new-job"]').first().click();
  await expect(page.locator("#job-dialog")).toBeVisible();
  await page.locator("#job-user").fill("browser-e2e");
  await page.locator("#job-name").fill(name);
  await page.locator("#job-cwd").fill(roots.default_path);
  await page.locator("#job-command").fill("echo browser-real");
  await page.locator("#job-description").fill("real browser to application journey");
  await page.locator("#new-job-form").getByRole("button", { name: "Create draft" }).click();
  await expect(page.getByText("Job created as DRAFT.")).toBeVisible();

  await page.locator('[data-route="jobs"]').click();
  await page.getByText(name).first().click();
  await expect(page.locator("#job-detail-title")).toHaveText(name);
  await expect(page.getByText("real browser to application journey")).toBeVisible();
  await page.mouse.click(20, 400);
  await expect(page.locator("#job-detail-dialog")).not.toBeVisible();
});

test("browser journey covers create, detail, description, queue, logs, and config", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "See what’s running and what’s next." })).toBeVisible();

  await page.locator('[data-route="jobs"]').click();
  await page.locator('[data-action="new-job"]').first().click();
  await page.locator("#job-user").fill("alice");
  await page.locator("#job-name").fill("browser-job");
  await page.locator("#job-cwd").fill("/workspace");
  await page.locator("#job-command").fill("echo browser");
  await page.locator("#job-description").fill("created in browser");
  await page.locator("#new-job-form").getByRole("button", { name: "Create draft" }).click();
  await expect(page.getByText("Job created as DRAFT.")).toBeVisible();

  await page.locator('[data-route="jobs"]').click();
  await page.getByText("browser-job").first().click();
  await expect(page.locator("#job-detail-title")).toHaveText("browser-job");
  await page.locator('[data-action="edit-description"]').click();
  await page.locator("#description-input").fill("updated in browser");
  await page.locator("#description-form").getByRole("button", { name: "Save" }).click();
  await expect(page.getByText("updated in browser")).toBeVisible();
  await page.locator('[data-job-action="commit"]').click();
  await page.locator("#confirm-accept").click();

  await page.locator('[data-route="queue"]').click();
  await page.locator('[data-queue-lock="true"]').click();
  await expect(page.getByText("Queue is locked")).toBeVisible();
  await page.locator('[data-queue-move="20000000-0000-4000-8000-000000000002"]').first().click();

  await page.locator('[data-route="logs"]').click();
  await page.locator("#log-job-select").selectOption("20000000-0000-4000-8000-000000000002");
  await expect(page.locator(".log-output")).toContainText("hello from stdout");

  await page.locator('[data-route="configuration"]').click();
  await page.locator("#timezone-input").fill("Asia/Tokyo");
  await page.locator('[data-timezone="Asia/Tokyo"]').click();
  await page.locator("#timezone-form").getByRole("button", { name: "Set timezone" }).click();
  await expect(page.locator(".config-summary strong")).toHaveText("Asia/Tokyo");
  await page.locator('[data-action="create-snapshot"]').click();
  await page.locator("[data-restore-path]").click();
  await page.locator("#confirm-accept").click();
  await expect(page.locator(".config-summary strong")).toHaveText("UTC");

  await page.locator('[data-route="policy"]').click();
  await expect(page.getByRole("heading", { name: "Execution policy" })).toBeVisible();
  await page.locator("#policy-max_bytes_per_job").fill("8");
  await page.locator('[data-policy-set="log-max-bytes-per-job"]').click();
  await expect(page.getByText("Policy updated.")).toBeVisible();
});

test("job owner suggestions close when the form is clicked elsewhere", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/#jobs");
  await page.locator('[data-action="new-job"]').first().click();
  await page.locator("#job-user").click();
  await expect(page.locator("#job-user-suggestions")).toHaveClass(/visible/);

  await page.locator("#job-name").click();
  await expect(page.locator("#job-user-suggestions")).not.toHaveClass(/visible/);
  await expect(page.locator("#job-user")).toHaveAttribute("aria-expanded", "false");
});

test("queue keeps scrolling inside the job list on desktop", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 800 });
  await mockBackend(page);
  await page.goto("/#queue");
  await expect(page.getByRole("heading", { name: "Execution queue" })).toBeVisible();

  await page.locator(".queue-order-table tbody").evaluate((tbody) => {
    const row = tbody.querySelector("tr");
    if (!row) throw new Error("Expected a queue row");
    for (let index = 0; index < 20; index += 1) tbody.append(row.cloneNode(true));
  });

  const overflow = await page.evaluate(() => {
    const main = document.querySelector(".main-content");
    const queue = document.querySelector(".queue-table");
    if (!(main instanceof HTMLElement) || !(queue instanceof HTMLElement)) {
      throw new Error("Expected queue layout elements");
    }
    return {
      mainClientHeight: main.clientHeight,
      mainScrollHeight: main.scrollHeight,
      queueClientHeight: queue.clientHeight,
      queueScrollHeight: queue.scrollHeight,
    };
  });

  expect(overflow.mainScrollHeight).toBe(overflow.mainClientHeight);
  expect(overflow.queueScrollHeight).toBeGreaterThan(overflow.queueClientHeight);
});

test("v1.3.1 layout contract and two-second refresh preserve active input", async ({ page }) => {
  let statusRequests = 0;
  let releaseNextPoll: () => void = () => undefined;
  const nextPoll = new Promise<void>((resolve) => { releaseNextPoll = resolve; });
  await mockBackend(page, {
    onRequest(path: string) {
      if (path === "/api/v1/status" && ++statusRequests >= 2) releaseNextPoll();
    },
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "See what’s running and what’s next." })).toBeVisible();
  await expect(page).toHaveTitle("Stoker");

  const visualContract = await page.evaluate(() => {
    const sidebar = document.querySelector(".sidebar");
    const appShell = document.querySelector(".app-shell");
    if (!(sidebar instanceof HTMLElement) || !(appShell instanceof HTMLElement)) {
      throw new Error("Expected app shell layout elements");
    }
    return {
      background: getComputedStyle(document.documentElement).getPropertyValue("--bg").trim(),
      sidebarWidth: sidebar.getBoundingClientRect().width,
      columns: getComputedStyle(appShell).gridTemplateColumns,
    };
  });
  expect(visualContract.background).toBe("#0b1018");
  expect(visualContract.sidebarWidth).toBe(248);
  expect(visualContract.columns.startsWith("248px ")).toBeTruthy();

  await page.locator('[data-route="jobs"]').click();
  await expect(page.getByRole("heading", { name: "All jobs" })).toBeVisible();
  const search = page.locator("#job-search");
  await search.pressSequentially("seed", { delay: 10 });
  await expect(search).toHaveValue("seed");
  await nextPoll;
  await expect(search).toHaveValue("seed");
  await expect(search).toBeFocused();

  const formPoll = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/v1/status");
  await page.locator('[data-action="new-job"]').click();
  await page.locator("#job-name").fill("unsaved draft");
  await formPoll;
  await expect(page.locator("#job-dialog")).toBeVisible();
  await expect(page.locator("#job-name")).toHaveValue("unsaved draft");
});

test("LAN mode loads without an authentication prompt", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "See what’s running and what’s next." })).toBeVisible();
  await expect(page.locator("dialog[open]")).toHaveCount(0);
});

test("policy page explains the queue gate and supports reset", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/#policy");
  await expect(page.getByRole("heading", { name: "Execution policy" })).toBeVisible();
  await expect(page.locator('[data-action="workspace-queue-lock"]')).toHaveText("Lock queue");
  await expect(page.locator('[data-policy-set="log-max-bytes-per-job"]')).toBeDisabled();

  await page.locator('[data-action="workspace-queue-lock"]').click();
  await expect(page.locator('[data-action="workspace-queue-lock"]')).toHaveText("Unlock queue");
  await expect(page.locator('[data-policy-set="log-max-bytes-per-job"]')).toBeEnabled();
  const runtime = page.locator("#policy-max_runtime_ms");
  await runtime.fill("0");
  await expect(page.getByText("Enter a positive whole number.")).toBeVisible();
  await expect(page.locator('[data-policy-set="max-runtime-ms"]')).toBeDisabled();
  await runtime.fill("1200");
  await page.locator('[data-policy-set="max-runtime-ms"]').click();
  await expect(page.getByText("Policy updated.")).toBeVisible();
  await page.locator('[data-policy-unset="max-runtime-ms"]').click();
  await expect(runtime).toHaveValue("");
});

test.describe("localized Web UI", () => {
  test("language menu supports keyboard selection, focus return and outside dismissal", async ({ page }, testInfo) => {
    await mockBackend(page);
    await page.goto("/");
    const trigger = page.locator(".topbar .language-picker");
    const menu = page.getByRole("menu");
    const english = page.getByRole("menuitemradio", { name: "English", exact: true });
    const chinese = page.getByRole("menuitemradio", { name: "繁體中文", exact: true });
    const japanese = page.getByRole("menuitemradio", { name: "日本語", exact: true });
    await trigger.focus();
    await trigger.press("ArrowDown");
    await expect(menu).toBeVisible();
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await expect(english).toBeFocused();
    await english.press("ArrowUp");
    await expect(japanese).toBeFocused();
    await japanese.press("Home");
    await expect(english).toBeFocused();
    await english.press("End");
    await expect(japanese).toBeFocused();
    await japanese.press("ArrowDown");
    await expect(english).toBeFocused();
    await english.press("ArrowDown");
    await chinese.press("Enter");
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-TW");
    await expect(menu).not.toBeVisible();
    await expect(trigger).toBeFocused();

    await trigger.click();
    await expect(chinese).toBeFocused();
    await expect(chinese).toHaveAttribute("aria-checked", "true");
    const menuBounds = (await menu.boundingBox())!;
    const buttonBounds = (await trigger.boundingBox())!;
    await page.screenshot({ path: testInfo.outputPath("language-menu.png"), clip: { x: menuBounds.x - 8, y: buttonBounds.y - 8, width: menuBounds.width + 16, height: menuBounds.y + menuBounds.height - buttonBounds.y + 16 } });
    await chinese.press("Escape");
    await expect(menu).not.toBeVisible();
    await expect(trigger).toBeFocused();

    await trigger.press("ArrowUp");
    await expect(japanese).toBeFocused();
    await japanese.press("e");
    await expect(english).toBeFocused();
    await page.getByRole("heading", { level: 1 }).click();
    await expect(menu).not.toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-TW");

    await trigger.click();
    await chinese.press("Tab");
    await expect(menu).not.toBeVisible();
    await expect(page.locator(".panel-link").first()).toBeFocused();
    await trigger.click();
    await trigger.click();
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await trigger.click();
    await page.locator('[data-route="jobs"]').focus();
    await expect(menu).not.toBeVisible();
  });

  for (const locale of ["zh-TW", "ja"] as const) {
    test(`${locale} covers all pages, dialogs, raw API values, and persistent preferences`, async ({ page }) => {
      const model = await mockBackend(page);
      const seed = model.jobs[0];
      seed.name = "使用者名稱・日本語";
      seed.description = "Original user text 原始描述";
      seed.state = "DRAFT";
      model.queue = [];
      await page.goto("/");
      const picker = page.locator(".topbar .language-picker");
      await chooseLanguage(page, locale);
      await expect(page.locator("html")).toHaveAttribute("lang", locale);
      await expect(page.getByRole("heading", { name: translate(locale, "overview.title") })).toBeVisible();
      await expect(page.locator(".activity-copy")).toContainText(translate(locale, "state.DRAFT"));

      await page.locator('[data-route="jobs"]').click();
      await expect(page.getByRole("heading", { name: translate(locale, "jobs.title") })).toBeVisible();
      await page.locator("#job-search").fill("使用者");
      await page.locator("#state-filter").selectOption("DRAFT");
      await chooseLanguage(page, "en");
      await chooseLanguage(page, locale);
      await expect(page.locator("#job-search")).toHaveValue("使用者");
      await expect(page.locator("#state-filter")).toHaveValue("DRAFT");
      await expect(page.locator('.jobs-table .state-badge')).toHaveText(translate(locale, "state.DRAFT"));

      await page.locator('[data-action="new-job"]').click();
      await page.locator("#job-name").fill("unsaved 草稿・下書き");
      await page.locator("#job-command").fill("echo original-command");
      await page.locator('[data-action="close-job-form"]').first().click();
      await chooseLanguage(page, "en");
      await chooseLanguage(page, locale);
      await page.locator('[data-action="new-job"]').click();
      await expect(page.locator("#job-name")).toHaveValue("unsaved 草稿・下書き");
      await expect(page.locator("#job-command")).toHaveValue("echo original-command");
      await expect(page.locator("#job-dialog-title")).toHaveText(translate(locale, "jobs.newTitle"));
      await page.locator('[data-action="close-job-form"]').first().click();

      await page.locator(".job-row").click();
      await expect(page.locator("#job-detail-title")).toHaveText(seed.name);
      await expect(page.locator(".job-description-preview")).toHaveText(seed.description!);
      await expect(page.locator(".job-command")).toHaveText("echo hello");
      await expect(page.locator(".job-detail-directory-status")).toHaveText("planned");
      await page.locator('[data-action="edit-description"]').click();
      await page.locator("#description-input").fill("unsaved edited description");
      await expect(page.locator("#description-input")).toHaveValue("unsaved edited description");
      await page.locator('[data-job-action="commit"]').click();
      await expect(page.locator("#confirm-title")).toHaveText(translate(locale, "confirm.commitTitle"));
      await page.locator("#confirm-accept").click();
      await expect(page.locator("#job-detail-dialog")).not.toBeVisible();
      expect(seed.state).toBe("QUEUED");
      await expect(page.locator("#toast-region")).toContainText(translate(locale, "toast.saved"));
      await chooseLanguage(page, "en");
      await expect(page.locator("#toast-region")).toContainText("Change saved to the server.");
      await chooseLanguage(page, locale);

      await page.locator('[data-route="queue"]').click();
      await expect(page.getByRole("heading", { name: translate(locale, "queue.title") })).toBeVisible();
      await expect(page.locator(".queue-order-table .state-badge").first()).toHaveText(translate(locale, "state.QUEUED"));
      await page.locator('[data-route="logs"]').click();
      await expect(page.getByRole("heading", { name: translate(locale, "logs.title") })).toBeVisible();
      await page.locator("#log-job-select").selectOption(seed.id);
      await expect(page.locator(".log-output")).toContainText("hello from stdout");
      await page.locator('[data-log-stream="stderr"]').click();
      await expect(page.locator(".log-message")).toContainText(translate(locale, "logs.unavailable", { stream: "stderr" }));

      await page.locator('[data-route="configuration"]').click();
      await expect(page.getByRole("heading", { name: translate(locale, "config.title") })).toBeVisible();
      await expect(page.locator("#timezone-feedback")).toHaveCount(0);
      await expect(page.locator("#timezone-set")).toBeDisabled();
      await page.locator("#timezone-input").fill("Not/AZone");
      await expect(page.locator("#timezone-feedback")).toHaveText(translate(locale, "config.chooseSuggestion"));
      await expect(page.locator("#timezone-set")).toBeDisabled();
      await page.locator("#timezone-input").fill("UTC");
      await expect(page.locator("#timezone-feedback")).toHaveCount(0);
      await expect(page.locator("#timezone-set")).toBeDisabled();
      await page.locator("#timezone-input").fill("Asia/Tokyo");
      await expect(page.locator("#timezone-feedback")).toHaveText(translate(locale, "config.validTimezone"));
      await expect(page.locator("#timezone-set")).toBeEnabled();
      await chooseLanguage(page, "en");
      await chooseLanguage(page, locale);
      await expect(page.locator("#timezone-input")).toHaveValue("Asia/Tokyo");
      await expect(page.locator(".config-summary strong")).toHaveText("UTC");

      await page.locator('[data-route="policy"]').click();
      await expect(page.getByRole("heading", { name: translate(locale, "policy.title"), level: 1 })).toBeVisible();
      await expect(page.locator('[data-action="workspace-queue-lock"]')).toHaveText(translate(locale, "queue.lock"));
      await page.locator('[data-action="workspace-queue-lock"]').click();
      await expect(page.locator('[data-action="workspace-queue-lock"]')).toHaveText(translate(locale, "queue.unlock"));
      await expect(page.locator("#policy-max_runtime_ms")).toBeEnabled();
      await page.locator("#policy-max_runtime_ms").fill("0");
      await expect(page.getByText(translate(locale, "policy.invalidPositive"))).toBeVisible();
      await page.locator("#policy-max_runtime_ms").fill("12345");
      await chooseLanguage(page, "en");
      await chooseLanguage(page, locale);
      await expect(page.locator("#policy-max_runtime_ms")).toHaveValue("12345");
      await page.reload();
      await picker.click();
      await expect(page.locator(`.language-option[data-locale="${locale}"]`)).toHaveAttribute("aria-checked", "true");
      await page.keyboard.press("Escape");
      await expect(page.locator("html")).toHaveAttribute("lang", locale);
      await expect(page.locator("#breadcrumb-current")).toHaveText(translate(locale, "nav.policy"));
    });
  }

  test("switching language does not reload data or reset the two-second polling timer", async ({ page }) => {
    let statusRequests = 0;
    await page.clock.install({ time: new Date("2026-01-01T00:00:00Z") });
    // Freeze before navigation so host/browser clock drift cannot move pauseAt into the past.
    await page.clock.pauseAt(new Date("2026-01-01T00:01:00Z"));
    await mockBackend(page, { onRequest(path) { if (path === "/api/v1/status") statusRequests++; } });
    await page.goto("/");
    await expect(page.locator("#connection-label")).toHaveText("Server connected");
    const before = statusRequests;
    await page.clock.runFor(1000);
    await chooseLanguage(page, "ja");
    await expect(page.locator("#connection-label")).toHaveText(translate("ja", "connection.connected"));
    expect(statusRequests).toBe(before);
    await page.clock.runFor(1000);
    await expect.poll(() => statusRequests).toBe(before + 1);
  });

  test("browser language detection and saved override work with unavailable storage", async ({ browser }) => {
    const context = await browser.newContext({ locale: "ja-JP" });
    try {
      const page = await context.newPage();
      await mockBackend(page);
      await page.goto("/");
      await expect(page.locator("html")).toHaveAttribute("lang", "ja");
      await chooseLanguage(page, "zh-TW");
      await page.reload();
      await expect(page.locator("html")).toHaveAttribute("lang", "zh-TW");
      await page.addInitScript(() => {
        Storage.prototype.getItem = () => { throw new DOMException("Storage unavailable", "SecurityError"); };
        Storage.prototype.setItem = () => { throw new DOMException("Storage unavailable", "SecurityError"); };
      });
      await page.reload();
      await expect(page.locator("html")).toHaveAttribute("lang", "ja");
      await chooseLanguage(page, "zh-TW");
      await expect(page.locator("html")).toHaveAttribute("lang", "zh-TW");
    } finally {
      await context.close();
    }
  });

  test("backend errors remain English under a translated failure screen", async ({ page }) => {
    await page.addInitScript(({ key }) => localStorage.setItem(key, "zh-TW"), { key: LANGUAGE_STORAGE_KEY });
    await mockBackend(page);
    await page.route("**/api/v1/status", route => typedError(route, 500, "internal", "Original backend error"));
    await page.goto("/");
    await expect(page.getByRole("heading", { name: translate("zh-TW", "workspace.loadFailed") })).toBeVisible();
    await expect(page.getByText("Original backend error")).toBeVisible();
    await expect(page.getByRole("button", { name: translate("zh-TW", "common.retry") })).toBeVisible();
  });

  for (const locale of ["zh-TW", "ja"] as const) {
    test(`${locale} fits navigation and controls at a 320px viewport`, async ({ page }, testInfo) => {
      await page.setViewportSize({ width: 320, height: 760 });
      await mockBackend(page);
      await page.goto("/");
      await chooseLanguage(page, locale);
      const titles: Record<string, StaticKey> = { overview: "overview.title", jobs: "jobs.title", queue: "queue.title", logs: "logs.title", configuration: "config.title", policy: "policy.title" };
      for (const [route, title] of Object.entries(titles)) {
        await page.locator(`[data-route="${route}"]`).click();
        await expect(page.getByRole("heading", { name: translate(locale, title), level: 1 })).toBeVisible();
        expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(320);
        const picker = page.locator(".topbar .language-picker");
        await expect(picker).toBeInViewport();
        const bounds = await picker.boundingBox();
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(320);
        await picker.click();
        const menu = page.getByRole("menu");
        await expect(menu).toBeInViewport();
        const menuBounds = (await menu.boundingBox())!;
        expect(menuBounds.x).toBeGreaterThanOrEqual(0);
        expect(menuBounds.x + menuBounds.width).toBeLessThanOrEqual(320);
        await page.keyboard.press("Escape");
        for (const label of await page.locator(".nav-label").all()) {
          const fits = await label.evaluate(element => {
            const labelBounds = element.getBoundingClientRect();
            const itemBounds = element.parentElement!.getBoundingClientRect();
            return labelBounds.left >= itemBounds.left && labelBounds.right <= itemBounds.right;
          });
          expect(fits).toBeTruthy();
        }
      }
      await page.screenshot({ path: testInfo.outputPath(`${locale}-mobile.png`), fullPage: true });
      await page.locator('[data-route="jobs"]').click();
      await page.locator('[data-action="new-job"]').click();
      await expect(page.locator("#job-dialog")).toBeVisible();
      await expect(page.locator("#job-dialog-title")).toHaveText(translate(locale, "jobs.newTitle"));
      expect((await page.locator("#job-dialog").boundingBox())!.width).toBe(320);
      expect((await page.locator("#job-cwd").boundingBox())!.height).toBeLessThan(60);
      await page.screenshot({ path: testInfo.outputPath(`${locale}-job-dialog.png`), fullPage: true });
    });
  }
});

async function chooseLanguage(page: Page, locale: Locale) {
  await page.locator(".topbar .language-picker").click();
  await page.locator(`.language-option[data-locale="${locale}"]`).click();
}

function requestJson<T>(request: Request): T {
  return request.postDataJSON() as T;
}

function timezone(model: MockModel): TimezoneInfo {
  return { name: model.timezone, source: "config" };
}

function status(model: MockModel): StatusResponse {
  return {
    scheduler: { running: false, pid: null, active_job: null, queued_jobs: model.queue.length },
    counts: {
      total: model.jobs.length,
      draft: model.jobs.filter((job) => job.state === "DRAFT").length,
      queued: model.jobs.filter((job) => job.state === "QUEUED").length,
      active: 0,
      succeeded: 0,
      failed: 0,
    },
    queue_locked: model.locked,
    timezone: timezone(model),
    generated_at: "2026-09-10T00:00:00Z",
  };
}

function configuration(model: MockModel): SettingsResponse {
  return {
    config: { timezone: model.timezone },
    effective_timezone: timezone(model),
    timezones: ["UTC", "Asia/Tokyo"],
    config_path: "/config/config.json",
    snapshot_dir: "/config/snapshots",
    snapshots: model.snapshots,
  };
}

function defaultPolicy(): PolicyResponse {
  const log = { max_bytes_per_job: 64, segment_bytes: 1, max_bytes_total: 1024, retention_jobs: 100, disk_reserve_bytes: 512 };
  const runtime = { termination_grace_ms: 500, max_runtime_ms: null, startup_timeout_ms: 30_000 };
  return {
    log,
    runtime,
    defaults: { log: { ...log }, runtime: { ...runtime } },
    units: { log: { max_bytes_per_job: "MB", segment_bytes: "MB", max_bytes_total: "MB", retention_jobs: "jobs", disk_reserve_bytes: "MB" }, runtime: { termination_grace_ms: "milliseconds", max_runtime_ms: "milliseconds", startup_timeout_ms: "milliseconds" } },
    queue_locked: false,
    can_update: false,
    active_jobs: [],
    blocked_reason: "Lock the queue before changing policy.",
  };
}

function setMockPolicyValue(policy: PolicyResponse, routeKey: string, value?: number): boolean {
  const section = routeKey.startsWith("log-") ? "log" : "runtime";
  const fieldKey = section === "log" ? routeKey.slice("log-".length) : routeKey;
  const key = fieldKey.replaceAll("-", "_") as keyof PolicyResponse["log"] & keyof PolicyResponse["runtime"];
  const target = policy[section] as Record<string, number | null>;
  const defaults = policy.defaults[section] as Record<string, number | null>;
  if (!(key in target)) return false;
  target[key] = value === undefined ? defaults[key] : value;
  return true;
}

function typedError(route: PlaywrightRoute, statusCode: number, code: string, message: string): Promise<void> {
  return route.fulfill({ status: statusCode, json: { error: message, code, message, details: null } });
}
