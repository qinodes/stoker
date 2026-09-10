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

function mockBackend(page, { requireToken = false } = {}) {
  const seed = baseJob({ id: "10000000-0000-4000-8000-000000000001" });
  const model = {
    jobs: [seed],
    queue: [seed],
    locked: false,
    timezone: "UTC",
    snapshots: [],
  };

  return page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    const method = request.method();
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
}

test("bundled browser creates and reads a job through the real Axum application stack", async ({ page, request }) => {
  const rootsResponse = await request.get("/api/v1/fs/roots");
  expect(rootsResponse.ok()).toBeTruthy();
  const roots = await rootsResponse.json();
  expect(roots.default_path).toBeTruthy();
  const name = `browser-real-${Date.now()}`;

  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Execution at a glance" })).toBeVisible();
  await page.locator('[data-action="new-job"]').first().click();
  await expect(page.locator("#job-dialog")).toBeVisible();
  await page.locator("#job-user").fill("browser-e2e");
  await page.locator("#job-name").fill(name);
  await page.locator("#job-cwd").fill(roots.default_path);
  await page.locator("#job-command").fill("echo browser-real");
  await page.locator("#job-description").fill("real browser to application journey");
  await page.locator("#job-form").getByRole("button", { name: "Create job" }).click();
  await expect(page.getByText("Job created as DRAFT.")).toBeVisible();

  await page.locator('[data-route="jobs"]').click();
  await page.getByText(name).first().click();
  await expect(page.locator("#job-detail-title")).toHaveText(name);
  await expect(page.getByText("real browser to application journey")).toBeVisible();
});

test("browser journey covers create, detail, description, queue, logs, and config", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Execution at a glance" })).toBeVisible();

  await page.locator('[data-action="new-job"]').first().click();
  await page.locator("#job-user").fill("alice");
  await page.locator("#job-name").fill("browser-job");
  await page.locator("#job-cwd").fill("/workspace");
  await page.locator("#job-command").fill("echo browser");
  await page.locator("#job-description").fill("created in browser");
  await page.locator("#job-form").getByRole("button", { name: "Create job" }).click();
  await expect(page.getByText("Job created as DRAFT.")).toBeVisible();

  await page.locator('[data-route="jobs"]').click();
  await page.getByText("browser-job").first().click();
  await expect(page.locator("#job-detail-title")).toHaveText("browser-job");
  await page.locator('[data-action="edit-description"]').click();
  await page.locator("#description-input").fill("updated in browser");
  await page.locator("#description-form").getByRole("button", { name: "Save description" }).click();
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
  await page.locator("#timezone-form").getByRole("button", { name: "Save timezone" }).click();
  await expect(page.getByText("Effective: Asia/Tokyo")).toBeVisible();
  await page.locator('[data-action="create-snapshot"]').click();
  await page.locator("[data-restore-path]").click();
  await page.locator("#confirm-accept").click();
  await expect(page.getByText("Effective: UTC")).toBeVisible();
});

test("LAN auth opens token dialog and retries with bearer credentials", async ({ page }) => {
  await mockBackend(page, { requireToken: true });
  await page.goto("/");
  await expect(page.locator("#token-dialog")).toBeVisible();
  await page.locator("#token-input").fill("browser-secret");
  await page.locator("#token-form").getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("heading", { name: "Execution at a glance" })).toBeVisible();
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
