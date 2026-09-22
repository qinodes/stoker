import type { Page, Request, Route } from "@playwright/test";
import type { PolicyResponse, ScheduledFlow, ScheduledRun, ScheduledStandaloneJob, WorkspaceMode } from "../../../src/types.ts";

export interface MockBackendOptions {
  workspaceMode: WorkspaceMode;
  maxConcurrency?: number;
  activeAttempts?: number;
  recoveryFence?: boolean;
  flow?: Partial<ScheduledFlow>;
  flowCount?: number;
}

export interface MockBackend {
  workspaceMode: WorkspaceMode;
  revisionConflict: boolean;
  sourceMode: "manual" | "sync";
  recoveryFence: boolean;
  maxConcurrency: number;
  activeAttempts: number;
  queueLocked: boolean;
}

const now = "2026-09-21T00:00:00Z";
const flowId = "nightly-flow";
const runId = "50000000-0000-4000-8000-000000000005";

/** Deterministic HTTP boundary for browser coverage; it never starts a Stoker server. */
export async function mockBackend(page: Page, options: MockBackendOptions): Promise<MockBackend> {
  const model: MockBackend = {
    workspaceMode: options.workspaceMode,
    revisionConflict: false,
    sourceMode: "manual",
    recoveryFence: options.recoveryFence ?? false,
    maxConcurrency: options.maxConcurrency ?? 2,
    activeAttempts: options.activeAttempts ?? 0,
    queueLocked: false,
  };
  const flows: ScheduledFlow[] = [{ ...flow(), ...options.flow }];
  for (let index = 2; index <= (options.flowCount ?? 1); index += 1) {
    flows.push({ ...flow(), flow_id: `flow-${index}`, name: `Flow ${index}` });
  }
  const appliedSchedules = new Map(flows.map((item) => [item.flow_id, item.schedule]));
  const jobs: ScheduledStandaloneJob[] = [standaloneJob()];
  const runs: ScheduledRun[] = [];

  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname;
    const method = request.method();
    if (path === "/api/v1/ui/config") return json(route, { version: "scheduled-browser-test" });
    if (path === "/api/v1/workspace") return json(route, workspace(model));
    if (path === "/api/v1/queue/lock" && method === "POST") { model.queueLocked = true; return json(route, { jobs: [], locked: true }); }
    if (path === "/api/v1/queue/unlock" && method === "POST") { model.queueLocked = false; return json(route, { jobs: [], locked: false }); }
    if (path === "/api/v1/workspace/mode" && method === "POST") {
      const target = requestJson<{ mode: WorkspaceMode }>(request).mode;
      if (!model.queueLocked) return typedError(route, 409, "conflict", "queue is unlocked; run 'stoker queue lock' first");
      if (model.activeAttempts > 0) return typedError(route, 409, "conflict", "cannot change mode while an execution is active");
      model.workspaceMode = target;
      return json(route, { mode: target });
    }
    if (path === "/api/v1/policy") return json(route, policy());
    if (path === "/api/v1/config" || path === "/api/v1/config/snapshots") return json(route, { config: { timezone: "UTC" }, effective_timezone: { name: "UTC", source: "config" }, timezones: ["UTC", "Asia/Tokyo"], config_path: "/config.json", snapshot_dir: "/snapshots", snapshots: [] });
    if (model.workspaceMode === "serial") return serial(route, path, method);
    if (!path.startsWith("/api/v1/scheduled/")) return typedError(route, 409, "mode_changed", "workspace mode changed", { mode: "scheduled" });

    if (path === "/api/v1/scheduled/overview") return json(route, {
      capacity: { max_concurrency: model.maxConcurrency, active_attempts: model.activeAttempts },
      flow_count: flows.length,
      live_flow_count: flows.filter((flow) => flow.committed).length,
      draft_flow_count: flows.filter((flow) => !flow.committed).length,
      active_runs: runs.filter((run) => run.state !== "RECOVERING" && !["CANCELLED", "SUCCEEDED", "FAILED", "FAILED_TO_START", "SKIPPED", "LOST"].includes(run.state)).map(summary),
      recovering_runs: runs.filter((run) => run.state === "RECOVERING").map(summary),
      recovery_fence: model.recoveryFence,
      next_occurrences: [], recent_failures: [],
    });
    if (path === "/api/v1/scheduled/flows" && method === "GET") return json(route, { flows });
    if (path === "/api/v1/scheduled/flows" && method === "POST") {
      const body = requestJson<Pick<ScheduledFlow, "flow_id" | "name" | "owner" | "schedule">>(request);
      const created = { ...flow(), ...body, tasks: [] };
      flows.push(created);
      return route.fulfill({ status: 201, json: { flow: created } });
    }
    if (path === "/api/v1/scheduled/jobs" && method === "GET") return json(route, { jobs });
    if (path === "/api/v1/scheduled/sources" && method === "GET") return json(route, { mode: model.sourceMode, revision: 1, hash: "sha256:source" });
    if (path === "/api/v1/scheduled/sources/mode" && method === "POST") {
      model.sourceMode = requestJson<{ mode: "manual" | "sync" }>(request).mode;
      return json(route, { mode: model.sourceMode, revision: 2, hash: "sha256:source" });
    }
    if (path === "/api/v1/scheduled/sources/dry-run" && method === "POST") return json(route, { hash: "sha256:reviewed", changed: true, revision: 2, diff: { added: 0, updated: 0, removed: flows.length, unchanged: 0 } });
    if (path === "/api/v1/scheduled/sources/sync" && method === "POST") {
      flows.splice(0);
      return json(route, { hash: "sha256:reviewed", changed: true, revision: 3, diff: { added: 0, updated: 0, removed: 1, unchanged: 0 } });
    }
    if (path === "/api/v1/scheduled/sources/export") return json(route, { document: { version: 1, flows: [] } });
    if (path === "/api/v1/scheduled/sources/snapshot") return json(route, { document: { version: 1, flows: [] }, path: "/snapshots/source.json" });
    if (path === "/api/v1/scheduled/settings/max-concurrency" && method === "PUT") {
      if (model.recoveryFence) return typedError(route, 409, "conflict", "recovery is required");
      model.maxConcurrency = requestJson<{ value: number }>(request).value;
      return json(route, { max_concurrency: model.maxConcurrency });
    }

    const flowMatch = path.match(/^\/api\/v1\/scheduled\/flows\/([^/]+)(.*)$/);
    if (flowMatch) return flowRoute(route, request, flows, appliedSchedules, runs, model, decodeURIComponent(flowMatch[1]), flowMatch[2]);
    const jobMatch = path.match(/^\/api\/v1\/scheduled\/jobs\/([^/]+)(.*)$/);
    if (jobMatch) return jobRoute(route, jobs, runs, decodeURIComponent(jobMatch[1]), jobMatch[2]);
    const runMatch = path.match(/^\/api\/v1\/scheduled\/runs\/([^/]+)(.*)$/);
    if (runMatch) return runRoute(route, runs, decodeURIComponent(runMatch[1]), runMatch[2]);
    const recoveryMatch = path.match(/^\/api\/v1\/scheduled\/recoveries\/([^/]+)\/reconcile$/);
    if (recoveryMatch && method === "POST") {
      model.recoveryFence = false;
      return json(route, { run: runs.find((run) => run.run_id === recoveryMatch[1]) || seededRun() });
    }
    return typedError(route, 404, "not_found", `unmocked ${method} ${path}`);
  });
  return model;
}

function flowRoute(route: Route, request: Request, flows: ScheduledFlow[], appliedSchedules: Map<string, ScheduledFlow["schedule"]>, runs: ScheduledRun[], model: MockBackend, id: string, suffix: string) {
  const index = flows.findIndex((entry) => entry.flow_id === id);
  const item = flows[index];
  if (!item) return typedError(route, 404, "not_found", "flow not found");
  if (suffix === "" && request.method() === "GET") return json(route, { flow: item });
  if (suffix === "" && request.method() === "DELETE") {
    if (item.committed) return typedError(route, 409, "conflict", "only an uncommitted draft flow can be deleted");
    flows.splice(index, 1);
    return route.fulfill({ status: 204 });
  }
  if (suffix === "/runs" && request.method() === "GET") return json(route, { runs: runs.filter((run) => run.flow_id === id).map(summary) });
  if (suffix === "/runs" && request.method() === "POST") {
    const run = seededRun(id);
    runs.splice(0, runs.length, run);
    return json(route, { flow: item });
  }
  if (suffix === "/tasks" && request.method() === "POST") {
    const body = requestJson<ScheduledFlow["tasks"][number]>(request);
    item.tasks.push({ ...body, sequence: item.tasks.length + 1 });
    item.draft_revision += 1;
    item.has_draft = true;
    return json(route, { flow: item });
  }
  const taskMatch = suffix.match(/^\/tasks\/([^/]+)$/);
  if (taskMatch && request.method() === "PATCH") {
    const task = item.tasks.find((entry) => entry.task_id === decodeURIComponent(taskMatch[1]));
    if (!task) return typedError(route, 404, "not_found", "task not found");
    Object.assign(task, requestJson<Partial<ScheduledFlow["tasks"][number]>>(request));
    item.draft_revision += 1;
    item.has_draft = true;
    return json(route, { flow: item });
  }
  if (suffix === "/apply" && request.method() === "POST") { appliedSchedules.set(id, item.schedule); item.frozen = false; item.has_draft = false; return json(route, { flow: item }); }
  if (suffix === "/unfreeze" && request.method() === "POST") {
    if (item.has_draft) return typedError(route, 409, "conflict", "a draft revision is required when a draft exists");
    item.frozen = false;
    return json(route, { flow: item });
  }
  if (suffix === "/discard" && request.method() === "POST") { item.schedule = appliedSchedules.get(id) ?? null; item.has_draft = false; return json(route, { flow: item }); }
  if (suffix === "/commit" && request.method() === "POST") {
    item.committed = true;
    item.enabled = true;
    item.draft_revision += 1;
    return json(route, { flow: item });
  }
  if (suffix === "/freeze" && request.method() === "POST") { item.frozen = true; return json(route, { flow: item }); }
  if (suffix === "/schedule" && request.method() === "PUT") {
    if (model.revisionConflict) return typedError(route, 409, "conflict", "stale draft revision");
    const body = requestJson<{ schedule?: ScheduledFlow["schedule"] }>(request);
    if (!body.schedule) return typedError(route, 422, "invalid_json", "missing field `schedule`");
    item.schedule = body.schedule;
    item.draft_revision += 1;
    item.has_draft = true;
    return json(route, { flow: item });
  }
  return json(route, { flow: item });
}

function jobRoute(route: Route, jobs: ScheduledStandaloneJob[], runs: ScheduledRun[], id: string, suffix: string) {
  const item = jobs.find((entry) => entry.job.id === id);
  if (!item) return typedError(route, 404, "not_found", "scheduled job not found");
  if (suffix === "" ) return json(route, { job: item });
  if (suffix === "/runs") return json(route, { runs: runs.filter((run) => run.flow_id === id).map(summary) });
  return json(route, { job: item });
}

function runRoute(route: Route, runs: ScheduledRun[], id: string, suffix: string) {
  const run = runs.find((entry) => entry.run_id === id) || seededRun();
  if (suffix === "") return json(route, { run });
  const logs = suffix.match(/^\/tasks\/([^/]+)\/attempts\/(\d+)\/logs$/);
  if (logs) return json(route, { stdout: "attempt output\\n", stderr: "", stdout_available: true, stderr_available: false, stdout_truncated: false, stderr_truncated: false, message: null });
  return json(route, { run });
}

function serial(route: Route, path: string, method: string) {
  if (path === "/api/v1/status") return json(route, { scheduler: { running: false, pid: null, active_job: null, queued_jobs: 0 }, counts: { total: 0, draft: 0, queued: 0, active: 0, succeeded: 0, failed: 0 }, queue_locked: false, timezone: { name: "UTC", source: "config" }, generated_at: now });
  if (path === "/api/v1/jobs") return json(route, { jobs: [], timezone: { name: "UTC", source: "config" } });
  if (path === "/api/v1/queue") return json(route, { jobs: [], locked: false });
  if (path === "/api/v1/policy") return json(route, policy());
  if (path === "/api/v1/config" || path === "/api/v1/config/snapshots") return json(route, { config: { timezone: "UTC" }, effective_timezone: { name: "UTC", source: "config" }, timezones: ["UTC", "Asia/Tokyo"], config_path: "/config.json", snapshot_dir: "/snapshots", snapshots: [] });
  if (path === "/api/v1/fs/roots") return json(route, { default_path: "/workspace", locations: [] });
  return typedError(route, 404, "not_found", `unmocked ${method} ${path}`);
}

function workspace(model: MockBackend) { return { mode: model.workspaceMode, queue_locked: model.queueLocked, recovery_fence: model.recoveryFence, scheduler: { running: true, pid: 7, active_job: null, queued_jobs: 0 }, timezone: { name: "UTC", source: "config" }, generated_at: now }; }
function flow(): ScheduledFlow { return { flow_id: flowId, name: "Nightly Flow", owner: "alice", mode: "scheduled", schedule: { kind: "once", at: "2026-09-22T00:00:00Z" }, tasks: [], committed: false, frozen: false, enabled: false, graph_revision: 1, schedule_generation: 0, draft_revision: 0, has_draft: false }; }
function standaloneJob(): ScheduledStandaloneJob { return { job: { id: "40000000-0000-4000-8000-000000000004", name: "Standalone scheduled job", user: "alice", cwd: "/workspace", command_line: "echo standalone", state: "DRAFT", created_at: now }, definition: { mode: "scheduled", schedule: null, retry: 0, enabled: false, generation: 0 }, flow_id: "standalone" }; }
function seededRun(id = flowId): ScheduledRun { return { run_id: runId, flow_id: id, state: "SUCCEEDED", generation: 1, source: "MANUAL", started_at: now, finished_at: now, tasks: [{ task_id: "build", state: "SUCCEEDED", attempt_count: 1, cancel_requested: false, attempts: [{ attempt_id: "60000000-0000-4000-8000-000000000006", number: 1, state: "SUCCEEDED", exit_code: 0, started_at: now, finished_at: now }] }] }; }
function summary(run: ScheduledRun) { return { run_id: run.run_id, flow_id: run.flow_id, state: run.state, started_at: run.started_at, finished_at: run.finished_at }; }
function policy(): PolicyResponse { const log = { max_bytes_per_job: 64, segment_bytes: 1, max_bytes_total: 1024, retention_jobs: 100, disk_reserve_bytes: 512 }; const runtime = { termination_grace_ms: 500, max_runtime_ms: null, startup_timeout_ms: 30_000 }; return { log, runtime, defaults: { log: { ...log }, runtime: { ...runtime } }, units: { log: { max_bytes_per_job: "MB", segment_bytes: "MB", max_bytes_total: "MB", retention_jobs: "jobs", disk_reserve_bytes: "MB" }, runtime: { termination_grace_ms: "milliseconds", max_runtime_ms: "milliseconds", startup_timeout_ms: "milliseconds" } }, queue_locked: false, can_update: false, active_jobs: [], blocked_reason: "Lock the queue before changing policy." }; }
function requestJson<T>(request: Request): T { return request.postDataJSON() as T; }
function json(route: Route, value: unknown) { return route.fulfill({ json: value }); }
function typedError(route: Route, status: number, code: string, message: string, details: unknown = null) { return route.fulfill({ status, json: { error: message, code, message, details } }); }
