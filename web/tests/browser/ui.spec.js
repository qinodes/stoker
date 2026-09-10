import { expect, test } from "@playwright/test";

const baseJob = (overrides = {}) => ({
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

async function mockBackend(page, { requireToken = false, onRequest = () => {} } = {}) {
  const seed = baseJob({ id: "10000000-0000-4000-8000-000000000001" });
  const model = {
    jobs: [seed],
    queue: [seed],
    locked: false,
    timezone: "UTC",
    snapshots: [],
  };

  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    const method = request.method();
    onRequest(path, method);
    if (path === "/api/v1/ui/config") {
      return route.fulfill({ json: {
        auth_required: requireToken,
        version: "1.3.1-test",
        max_job_name_length: 128,
        max_job_user_length: 50,
        max_job_description_length: 200,
      } });
    }
    if (requireToken && request.headers().authorization !== "Bearer browser-secret") {
      return route.fulfill({ status: 401, json: {
        error: "legacy text",
        code: "unauthorized",
        message: "token required",
        details: null,
      } });
    }
    if (path === "/api/v1/status") return route.fulfill({ json: status(model) });
    if (path === "/api/v1/jobs" && method === "GET") {
      return route.fulfill({ json: { jobs: model.jobs, timezone: timezone(model) } });
    }
    if (path === "/api/v1/jobs" && method === "POST") {
      const body = request.postDataJSON();
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
        const body = request.postDataJSON();
        job.description = body.description;
        job.description_revision += 1;
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
      model.queue.splice(request.postDataJSON().target_order - 1, 0, job);
      model.queue.forEach((entry, order) => { entry.queue_order = order + 1; });
      return route.fulfill({ json: { jobs: model.queue, locked: model.locked } });
    }
    if (path === "/api/v1/config/timezone" && method === "PUT") {
      model.timezone = request.postDataJSON().value;
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
  const roots = await rootsResponse.json();
  expect(roots.default_path).toBeTruthy();
  const name = `browser-real-${Date.now()}`;

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "See what’s running and what’s next." })).toBeVisible();
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
  let releaseNextPoll;
  const nextPoll = new Promise((resolve) => { releaseNextPoll = resolve; });
  await mockBackend(page, {
    onRequest(path) {
      if (path === "/api/v1/status" && ++statusRequests >= 2) releaseNextPoll();
    },
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "See what’s running and what’s next." })).toBeVisible();

  const visualContract = await page.evaluate(() => ({
    background: getComputedStyle(document.documentElement).getPropertyValue("--bg").trim(),
    sidebarWidth: document.querySelector(".sidebar").getBoundingClientRect().width,
    columns: getComputedStyle(document.querySelector(".app-shell")).gridTemplateColumns,
  }));
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

test("LAN auth opens token dialog and retries with bearer credentials", async ({ page }) => {
  await mockBackend(page, { requireToken: true });
  await page.goto("/");
  await expect(page.locator("#token-dialog")).toBeVisible();
  await page.locator("#token-input").fill("browser-secret");
  await page.locator("#token-form").getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("heading", { name: "See what’s running and what’s next." })).toBeVisible();
});

function timezone(model) {
  return { name: model.timezone, source: "config" };
}

function status(model) {
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

function configuration(model) {
  return {
    config: { timezone: model.timezone },
    effective_timezone: timezone(model),
    timezones: ["UTC", "Asia/Tokyo"],
    config_path: "/config/config.json",
    snapshot_dir: "/config/snapshots",
    snapshots: model.snapshots,
  };
}

function typedError(route, statusCode, code, message) {
  return route.fulfill({ status: statusCode, json: { error: message, code, message, details: null } });
}
