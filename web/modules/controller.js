import { createApiClient } from "./api-client.js";
import { toast } from "./components.js";
import { escapeAttribute, escapeHtml, routeTitle } from "./formatters.js";
import {
  applyLoad,
  beginLoad,
  cacheDirectory,
  cachedDirectory,
  createState,
  failLoad,
} from "./state.js";
import { renderConfiguration } from "./views/configuration.js";
import { renderJobDetail, renderJobForm, renderJobs } from "./views/jobs.js";
import { renderLogs } from "./views/logs.js";
import { renderOverview } from "./views/overview.js";
import { renderQueue } from "./views/queue.js";
const ROUTES = ["overview", "jobs", "queue", "logs", "configuration"];

export function bootstrap() {
  const state = createState(location.hash.slice(1) || "overview");
  const elements = collectElements();
  const api = createApiClient({ onUnauthorized: () => showTokenDialog(state, elements) });
  let confirmationResolver = null;
  let detailEditing = false;
  function render() {
    if (!state.loaded) return;
    const focus = captureFocus();
    state.route = ROUTES.includes(state.route) ? state.route : "overview";
    elements.app.dataset.view = state.route;
    document.querySelectorAll("[data-route]").forEach((item) => item.classList.toggle("active", item.dataset.route === state.route));
    document.getElementById("breadcrumb-current").textContent = routeTitle(state.route);
    const views = { overview: renderOverview, jobs: renderJobs, queue: renderQueue, logs: renderLogs, configuration: renderConfiguration };
    elements.app.innerHTML = views[state.route](state);
    updateChrome(state);
    restoreFocus(focus);
  }
  async function loadData({ forceRender = false } = {}) {
    const sequence = beginLoad(state);
    if (!state.loaded) elements.app.innerHTML = '<div class="page-loading"><span class="spinner"></span><span>Loading workspace…</span></div>';
    try {
      const configPromise = state.config ? Promise.resolve(state.config) : api.get("/api/v1/ui/config");
      const [config, status, jobs, queue, settings] = await Promise.all([
        configPromise,
        api.get("/api/v1/status"),
        api.get("/api/v1/jobs"),
        api.get("/api/v1/queue"),
        api.get("/api/v1/config"),
      ]);
      if (!applyLoad(state, sequence, { config, status, jobs, queue, settings })) return;
      document.getElementById("app-version").textContent = config.version || "—";
      updateConnection(true);
      if (state.logs.jobId) await loadLogs(false);
      if (state.selectedJob) {
        state.selectedJob = state.jobs.find((job) => job.id === state.selectedJob.id) || state.selectedJob;
        if (elements.jobDetailDialog.open && !detailEditing) renderDetail();
      }
      if (forceRender || !isInteractiveEditing(elements, detailEditing)) render();
      else {
        updateChrome(state);
        refreshLiveTimes(state.timezone?.name);
      }
    } catch (error) {
      if (!failLoad(state, sequence, error)) return;
      updateConnection(false);
      if (state.loaded) showToast(error.message, true);
      else {
        elements.app.innerHTML = `<div class="page-heading"><div><div class="eyebrow">Workspace unavailable</div><h1>Couldn’t load Stoker</h1><p>${escapeText(error.message)}</p></div><button class="button primary" data-action="refresh">Try again</button></div>`;
      }
    }
  }
  async function mutate(path, method, body = null, message = "Change saved to the server.") {
    try {
      const result = await api.send(path, method, body);
      showToast(message);
      await loadData();
      return result;
    } catch (error) {
      showToast(error.message, true);
      await loadData();
      return null;
    }
  }
  async function openJobForm() {
    try {
      state.filesystem.roots ||= await api.get("/api/v1/fs/roots");
      state.jobDraft.cwd ||= state.filesystem.roots.default_path || "";
      state.filesystem.inputPath = state.jobDraft.cwd;
    } catch (error) {
      state.filesystem.error = error.message;
    }
    elements.jobDialogContent.innerHTML = renderJobForm(state);
    if (!elements.jobDialog.open) elements.jobDialog.showModal();
  }

  async function loadDirectory(path) {
    if (!path) return;
    const cached = cachedDirectory(state, path);
    if (cached) {
      state.filesystem.current = cached;
      state.filesystem.inputPath = cached.path;
      elements.jobDialogContent.innerHTML = renderJobForm(state);
      return;
    }
    const requestId = ++state.filesystem.requestId;
    state.filesystem.loading = true;
    state.filesystem.error = "";
    elements.jobDialogContent.innerHTML = renderJobForm(state);
    try {
      const value = await api.get(`/api/v1/fs/directories?path=${encodeURIComponent(path)}`);
      if (requestId !== state.filesystem.requestId) return;
      cacheDirectory(state, value.path, value);
      state.filesystem.current = value;
      state.filesystem.inputPath = value.path;
    } catch (error) {
      if (requestId === state.filesystem.requestId) state.filesystem.error = error.message;
    } finally {
      if (requestId === state.filesystem.requestId) {
        state.filesystem.loading = false;
        elements.jobDialogContent.innerHTML = renderJobForm(state);
      }
    }
  }

  async function openJobDetail(id) {
    const requestId = ++state.detailRequestId;
    elements.jobDetailContent.innerHTML = '<div class="page-loading"><span class="spinner"></span><span>Loading job…</span></div>';
    if (!elements.jobDetailDialog.open) elements.jobDetailDialog.showModal();
    try {
      const detail = await api.get(`/api/v1/jobs/${id}`);
      if (requestId !== state.detailRequestId) return;
      state.selectedJob = detail.job;
      state.selectedJobDetail = detail;
      detailEditing = false;
      renderDetail();
    } catch (error) {
      if (requestId === state.detailRequestId) elements.jobDetailContent.innerHTML = `<div class="form-feedback invalid">${escapeText(error.message)}</div>`;
    }
  }

  function renderDetail() {
    elements.jobDetailContent.innerHTML = renderJobDetail(state.selectedJob, state, detailEditing);
  }

  async function loadLogs(renderAfter = true) {
    if (!state.logs.jobId) {
      state.logs.data = null;
      if (renderAfter) render();
      return;
    }
    try {
      state.logs.data = await api.get(`/api/v1/jobs/${state.logs.jobId}/logs`);
      state.logs.error = null;
    } catch (error) {
      state.logs.data = null;
      state.logs.error = error.message;
    }
    if (renderAfter) render();
  }

  elements.app.addEventListener("click", async (event) => {
    const target = event.target.closest("button, [data-job-open]");
    if (!target) return;
    const action = target.dataset.action;
    if (action === "new-job") await openJobForm();
    else if (action === "refresh") await loadData();
    else if (action === "clean-jobs" && await confirmAction("Maintenance", "Clean terminal jobs?", "This removes terminal job history and its run artifacts.", "Clean jobs", true)) await mutate("/api/v1/clean", "POST", null, "Terminal job history cleaned.");
    else if (target.dataset.jobOpen) await openJobDetail(target.dataset.jobOpen);
    else if (target.dataset.pageKind) {
      state.pagination[target.dataset.pageKind] = Number(target.dataset.pageNumber || target.dataset.page);
      render();
    } else if (target.dataset.queueLock) await mutate(`/api/v1/queue/${target.dataset.queueLock === "true" ? "lock" : "unlock"}`, "POST");
    else if (target.dataset.queueMove) await mutate(`/api/v1/queue/${target.dataset.queueMove}/move`, "POST", { target_order: Number(target.dataset.targetOrder) });
    else if (target.dataset.logStream) {
      state.logs.stream = target.dataset.logStream;
      render();
    } else if (target.dataset.timezone) {
      const input = document.getElementById("timezone-input");
      input.value = target.dataset.timezone;
      state.configurationDraft = input.value;
      updateTimezonePicker(state);
      input.focus({ preventScroll: true });
    } else if (action === "create-snapshot") await mutate("/api/v1/config/snapshot", "POST", null, "Configuration snapshot created.");
    else if (action === "unset-timezone") {
      state.configurationDraft = null;
      await mutate("/api/v1/config/timezone", "DELETE", null, "System timezone enabled.");
    }
    else if (target.dataset.restorePath && await confirmAction("Configuration snapshot", "Restore this snapshot?", "The current configuration will be preserved before restore.", "Restore snapshot")) await mutate("/api/v1/config/restore", "POST", { path: target.dataset.restorePath });
  });

  elements.app.addEventListener("input", (event) => {
    if (event.target.id === "job-search") {
      state.filters.search = event.target.value;
      state.pagination.jobs = 1;
      render();
    } else if (event.target.id === "log-job-search") {
      state.logs.search = event.target.value;
      render();
    } else if (event.target.id === "timezone-input") {
      state.configurationDraft = event.target.value;
      updateTimezonePicker(state);
    }
  });

  elements.app.addEventListener("change", async (event) => {
    if (event.target.id === "user-filter" || event.target.id === "job-owner-filter") state.filters.user = event.target.value;
    else if (event.target.id === "state-filter" || event.target.id === "job-state-filter") state.filters.state = event.target.value;
    else if (event.target.id === "log-job-select") {
      state.logs.jobId = event.target.value;
      await loadLogs();
      return;
    } else return;
    state.pagination.jobs = 1;
    render();
  });

  elements.app.addEventListener("submit", async (event) => {
    if (event.target.id !== "timezone-form") return;
    event.preventDefault();
    const result = await mutate("/api/v1/config/timezone", "PUT", { value: document.getElementById("timezone-input").value });
    if (result) {
      state.configurationDraft = null;
      await loadData({ forceRender: true });
    }
  });

  elements.jobDialogContent.addEventListener("input", (event) => {
    captureDraft();
    if (event.target.id === "job-description") {
      const count = document.getElementById("job-description-count");
      if (count) count.textContent = `${[...event.target.value].length}/${state.config?.max_job_description_length || 200}`;
    } else if (event.target.id === "job-user") {
      updateOwnerSuggestions(state);
    }
  });
  elements.jobDialogContent.addEventListener("focusin", (event) => {
    if (event.target.id === "job-user") updateOwnerSuggestions(state);
  });
  elements.jobDialogContent.addEventListener("click", async (event) => {
    const target = event.target.closest("button");
    if (!target) return;
    captureDraft();
    if (target.dataset.action === "close-job-form") elements.jobDialog.close();
    else if (target.dataset.jobUser) {
      state.jobDraft.user = target.dataset.jobUser;
      const input = document.getElementById("job-user");
      input.value = state.jobDraft.user;
      document.getElementById("job-user-suggestions").classList.remove("visible");
      input.setAttribute("aria-expanded", "false");
      input.focus({ preventScroll: true });
    }
    else if (target.dataset.action === "browse-directory") await loadDirectory(state.jobDraft.cwd || state.filesystem.roots?.default_path);
    else if (target.dataset.action === "open-directory") await loadDirectory(document.getElementById("fs-path")?.value);
    else if (target.dataset.directory) await loadDirectory(target.dataset.directory);
    else if (target.dataset.action === "choose-directory" && state.filesystem.current) {
      state.jobDraft.cwd = state.filesystem.current.path;
      state.filesystem.roots = null;
      elements.jobDialogContent.innerHTML = renderJobForm(state);
    }
  });

  elements.jobDialogContent.addEventListener("submit", async (event) => {
    if (event.target.id !== "new-job-form") return;
    event.preventDefault();
    captureDraft();
    const created = await mutate("/api/v1/jobs", "POST", state.jobDraft, "Job created as DRAFT.");
    if (created) {
      state.jobDraft = { user: state.jobDraft.user, name: "", cwd: state.jobDraft.cwd, command: "", description: "" };
      elements.jobDialog.close();
    }
  });

  elements.jobDetailContent.addEventListener("click", async (event) => {
    const target = event.target.closest("button");
    if (!target) return;
    if (target.dataset.action === "close-job-detail") elements.jobDetailDialog.close();
    else if (target.dataset.action === "copy-job-id") {
      await navigator.clipboard.writeText(target.dataset.jobId);
      target.classList.add("copied");
    }
    else if (target.dataset.action === "edit-description") {
      detailEditing = !detailEditing;
      renderDetail();
    } else if (target.dataset.action === "view-job-logs") {
      state.logs.jobId = target.dataset.jobId;
      location.hash = "logs";
      elements.jobDetailDialog.close();
      await loadLogs();
    } else if (target.dataset.jobAction) {
      const action = target.dataset.jobAction;
      if (await confirmAction("Job action", `${action === "commit" ? "Commit" : "Cancel"} this job?`, "The scheduler will apply this state transition.", action === "commit" ? "Commit job" : "Cancel job", action === "cancel")) {
        await mutate(`/api/v1/jobs/${target.dataset.jobId}/${action}`, "POST");
        elements.jobDetailDialog.close();
      }
    }
  });

  elements.jobDetailContent.addEventListener("submit", async (event) => {
    if (event.target.id !== "description-form") return;
    event.preventDefault();
    const updated = await mutate(`/api/v1/jobs/${state.selectedJob.id}/description`, "PATCH", { description: document.getElementById("description-input").value, expected_revision: state.selectedJob.description_revision }, "Description updated.");
    if (updated) {
      state.selectedJob = updated.job;
      detailEditing = false;
      renderDetail();
    }
  });

  // The drawers occupy the right side of the viewport. A click on the native
  // dialog element itself means the user clicked its backdrop; clicks inside
  // the shell bubble from a child and must leave the drawer open.
  elements.jobDialog.addEventListener("click", (event) => {
    if (event.target === elements.jobDialog) elements.jobDialog.close();
  });
  elements.jobDetailDialog.addEventListener("click", (event) => {
    if (event.target === elements.jobDetailDialog) elements.jobDetailDialog.close();
  });

  document.getElementById("refresh-button").addEventListener("click", () => loadData());
  window.addEventListener("hashchange", () => {
    state.route = location.hash.slice(1) || "overview";
    if (state.loaded) render();
  });
  elements.tokenForm.addEventListener("submit", (event) => {
    event.preventDefault();
    const token = document.getElementById("token-input").value.trim();
    if (!token) return;
    sessionStorage.setItem("stoker-ui-token", token);
    elements.tokenDialog.close();
    loadData();
  });
  elements.confirmCancel.addEventListener("click", () => closeConfirmation(false));
  elements.confirmAccept.addEventListener("click", () => closeConfirmation(true));
  elements.confirmDialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    closeConfirmation(false);
  });

  function confirmAction(kicker, title, message, acceptLabel, destructive = false) {
    return new Promise((resolve) => {
      confirmationResolver = resolve;
      document.getElementById("confirm-kicker").textContent = kicker;
      document.getElementById("confirm-title").textContent = title;
      document.getElementById("confirm-message").textContent = message;
      elements.confirmAccept.textContent = acceptLabel;
      elements.confirmAccept.classList.toggle("danger", destructive);
      elements.confirmAccept.classList.toggle("primary", !destructive);
      elements.confirmDialog.showModal();
    });
  }

  function closeConfirmation(value) {
    if (!confirmationResolver) return;
    const resolve = confirmationResolver;
    confirmationResolver = null;
    elements.confirmDialog.close();
    resolve(value);
  }

  function captureDraft() {
    for (const field of ["user", "name", "cwd", "command", "description"]) {
      const input = document.getElementById(`job-${field}`);
      if (input) state.jobDraft[field] = input.value;
    }
  }

  function showToast(message, error = false) {
    const element = toast(message, error);
    document.getElementById("toast-region").appendChild(element);
    setTimeout(() => element.remove(), 4200);
  }

  loadData();
  setInterval(() => {
    if (document.visibilityState === "visible") loadData();
  }, 2000);
}

function collectElements() {
  return {
    app: document.getElementById("app"),
    tokenDialog: document.getElementById("token-dialog"),
    tokenForm: document.getElementById("token-form"),
    confirmDialog: document.getElementById("confirm-dialog"),
    confirmCancel: document.getElementById("confirm-cancel"),
    confirmAccept: document.getElementById("confirm-accept"),
    jobDialog: document.getElementById("job-dialog"),
    jobDialogContent: document.getElementById("job-dialog-content"),
    jobDetailDialog: document.getElementById("job-detail-dialog"),
    jobDetailContent: document.getElementById("job-detail-content"),
  };
}

function showTokenDialog(state, elements) {
  state.error = "This UI requires a valid LAN access token.";
  if (!elements.tokenDialog.open) elements.tokenDialog.showModal();
}

function updateChrome(state) {
  const running = Boolean(state.status?.scheduler.running);
  const pill = document.getElementById("scheduler-pill");
  pill.classList.toggle("running", running);
  pill.classList.toggle("stopped", !running);
  document.getElementById("scheduler-label").textContent = running ? "Scheduler running" : "Scheduler stopped";
}

function updateConnection(online) {
  const dot = document.getElementById("connection-dot");
  dot.classList.toggle("online", online);
  dot.classList.toggle("error", !online);
  document.getElementById("connection-label").textContent = online ? "Server connected" : "Connection issue";
  document.getElementById("connection-detail").textContent = online ? "State synced just now" : "Retry to reconnect";
}

export function isInteractiveEditing(elements, detailEditing = false) {
  const tagName = document.activeElement?.tagName;
  return ["INPUT", "SELECT", "TEXTAREA"].includes(tagName)
    || elements.jobDialog.open
    || elements.confirmDialog.open
    || detailEditing;
}

function refreshLiveTimes(timezone) {
  document.querySelectorAll("time.live-time").forEach((element) => {
    const value = element.getAttribute("datetime");
    if (!value) return;
    const date = new Date(value);
    if (Number.isNaN(date.valueOf())) return;
    const options = { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" };
    if (timezone) options.timeZone = timezone;
    try {
      element.textContent = new Intl.DateTimeFormat(undefined, options).format(date);
    } catch {
      delete options.timeZone;
      element.textContent = new Intl.DateTimeFormat(undefined, options).format(date);
    }
  });
}

function updateTimezonePicker(state) {
  const input = document.getElementById("timezone-input");
  const feedback = document.getElementById("timezone-feedback");
  const submit = document.getElementById("timezone-set");
  const suggestions = document.getElementById("timezone-suggestions");
  if (!input || !feedback || !submit || !suggestions) return;
  const value = input.value.trim();
  const saved = state.settings?.config?.timezone || "";
  const timezones = state.settings?.timezones || [];
  const exact = timezones.includes(value);
  const changed = value !== saved;
  submit.disabled = !exact || !changed;
  feedback.className = `form-feedback ${exact ? "valid" : value ? "invalid" : ""}`;
  feedback.textContent = exact && changed ? "✓ Valid timezone. Ready to save." : exact ? "Current timezone is already selected." : value ? "Choose a timezone from the suggestions." : "Start typing to search available timezones.";
  const query = value.toLowerCase();
  const matches = query ? timezones.filter((zone) => zone.toLowerCase().includes(query)).slice(0, 8) : [];
  suggestions.innerHTML = matches.map((zone) => `<button class="timezone-option" type="button" role="option" data-timezone="${escapeAttribute(zone)}">${escapeHtml(zone)}</button>`).join("");
  const visible = matches.length > 0 && document.activeElement === input;
  suggestions.classList.toggle("visible", visible);
  input.setAttribute("aria-expanded", String(visible));
}

function updateOwnerSuggestions(state) {
  const input = document.getElementById("job-user");
  const suggestions = document.getElementById("job-user-suggestions");
  if (!input || !suggestions) return;
  const query = input.value.trim().toLowerCase();
  const owners = [...new Set(state.jobs.map((job) => job.user).filter(Boolean))]
    .filter((owner) => !query || owner.toLowerCase().includes(query))
    .sort()
    .slice(0, 50);
  suggestions.innerHTML = owners.length ? `<div class="job-suggestions-label">Known owners · ${owners.length}</div>${owners.map((owner) => `<button class="job-suggestion" type="button" role="option" data-job-user="${escapeAttribute(owner)}"><span class="suggestion-avatar">${escapeHtml(owner.slice(0, 1).toUpperCase())}</span><span>${escapeHtml(owner)}</span></button>`).join("")}` : "";
  const visible = owners.length > 0 && document.activeElement === input;
  suggestions.classList.toggle("visible", visible);
  input.setAttribute("aria-expanded", String(visible));
}

function captureFocus() {
  const focused = document.activeElement;
  return focused && focused.id ? {
    id: focused.id,
    start: typeof focused.selectionStart === "number" ? focused.selectionStart : null,
    end: typeof focused.selectionEnd === "number" ? focused.selectionEnd : null,
  } : null;
}

function restoreFocus(snapshot) {
  if (!snapshot) return;
  const input = document.getElementById(snapshot.id);
  if (!input) return;
  input.focus({ preventScroll: true });
  if (snapshot.start !== null && snapshot.end !== null && typeof input.setSelectionRange === "function") {
    const end = Math.min(snapshot.end, input.value.length);
    input.setSelectionRange(Math.min(snapshot.start, end), end);
  }
}

function escapeText(value) {
  const element = document.createElement("span");
  element.textContent = value;
  return element.innerHTML;
}
