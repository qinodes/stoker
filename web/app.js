(() => {
  "use strict";

  const MAX_LOG_KB = 256;
  const MAX_OWNER_SUGGESTIONS = 50;
  const DIRECTORY_CACHE_TTL_MS = 20_000;
  const DIRECTORY_CACHE_LIMIT = 64;
  const JOBS_PAGE_SIZE = 6;
  const SNAPSHOTS_PAGE_SIZE = 5;

  const state = {
    config: null,
    settings: null,
    status: null,
    timezone: null,
    jobs: [],
    queue: { jobs: [], locked: false },
    logs: { jobId: "", stream: "stdout", data: null, error: null, jobSearch: "" },
    route: location.hash.slice(1) || "overview",
    filters: { search: "", user: "", state: "" },
    pagination: { jobs: 1, snapshots: 1 },
    loaded: false,
    loading: false,
    renderAfterLoad: false,
    configurationDraft: null,
    jobDraft: { user: "", name: "", cwd: "", command: "" },
    jobDialogOpen: false,
    jobDialogView: "form",
    filesystem: { roots: null, current: null, inputPath: "", loading: false, error: "", requestId: 0, directoryCache: new Map(), directoryRequests: new Map() },
    jobDetails: { selectedId: "", job: null, workingDirectoryStatus: "", displayTimezone: "", loading: false, action: "", error: "", requestId: 0 },
    error: null,
  };

  const app = document.getElementById("app");
  const tokenDialog = document.getElementById("token-dialog");
  const tokenForm = document.getElementById("token-form");
  const confirmDialog = document.getElementById("confirm-dialog");
  const confirmKicker = document.getElementById("confirm-kicker");
  const confirmTitle = document.getElementById("confirm-title");
  const confirmMessage = document.getElementById("confirm-message");
  const confirmCancel = document.getElementById("confirm-cancel");
  const confirmAccept = document.getElementById("confirm-accept");
  const jobDialog = document.getElementById("job-dialog");
  const jobDialogContent = document.getElementById("job-dialog-content");
  const jobDetailDialog = document.getElementById("job-detail-dialog");
  const jobDetailContent = document.getElementById("job-detail-content");
  let confirmationResolver = null;

  function getToken() {
    const hash = new URLSearchParams(location.hash.slice(1));
    const fragmentToken = hash.get("token");
    if (fragmentToken) {
      sessionStorage.setItem("stoker-ui-token", fragmentToken);
      history.replaceState(null, "", location.pathname + location.search + "#overview");
    }
    return sessionStorage.getItem("stoker-ui-token") || "";
  }

  function headers() {
    const token = getToken();
    return token ? { Authorization: `Bearer ${token}` } : {};
  }

  async function request(path, options = {}) {
    const requestHeaders = { ...headers(), ...(options.headers || {}) };
    if (options.body && !requestHeaders["Content-Type"]) requestHeaders["Content-Type"] = "application/json";
    const response = await fetch(path, { ...options, headers: requestHeaders, cache: "no-store" });
    if (response.status === 401) {
      state.error = "This UI requires a valid LAN access token.";
      if (state.config && state.config.auth_required && tokenDialog && !tokenDialog.open) tokenDialog.showModal();
      throw new Error("Unauthorized");
    }
    if (!response.ok) {
      let message = `Request failed (${response.status})`;
      try { message = (await response.json()).error || message; } catch (_) { /* keep status */ }
      throw new Error(message);
    }
    return response.json();
  }

  function openConfirmation({ kicker, title, message, acceptLabel, destructive = false }) {
    return new Promise((resolve) => {
      confirmationResolver = resolve;
      confirmKicker.textContent = kicker;
      confirmTitle.textContent = title;
      confirmMessage.textContent = message;
      confirmAccept.textContent = acceptLabel;
      confirmAccept.classList.toggle("primary", !destructive);
      confirmAccept.classList.toggle("danger", destructive);
      confirmDialog.showModal();
      confirmAccept.focus({ preventScroll: true });
    });
  }

  function closeConfirmation(confirmed) {
    if (!confirmationResolver) return;
    const resolve = confirmationResolver;
    confirmationResolver = null;
    confirmDialog.close();
    resolve(confirmed);
  }

  async function mutate(path, method, body = null, afterSuccess = null) {
    try {
      const response = await request(path, {
        method,
        body: body === null ? undefined : JSON.stringify(body),
      });
      const successMessage = afterSuccess ? afterSuccess(response) : null;
      showToast(successMessage || "Change saved to the server.");
      await loadData();
    } catch (error) {
      showToast(error.message, true);
      await loadData();
    }
  }

  async function loadData(options = {}) {
    if (state.loading) {
      if (options.forceRender) state.renderAfterLoad = true;
      return;
    }
    state.loading = true;
    state.error = null;
    if (!state.loaded) renderLoading();
    try {
      if (!state.config) state.config = await request("/api/v1/ui/config");
      const [status, jobs, queue] = await Promise.all([
        request("/api/v1/status"),
        request("/api/v1/jobs"),
        request("/api/v1/queue"),
      ]);
      state.settings = await request("/api/v1/config");
      state.status = status;
      state.jobs = jobs.jobs || [];
      state.timezone = jobs.timezone || status.timezone || null;
      state.queue = queue;
      if (state.jobDetails.selectedId && !state.jobDetails.action) {
        const currentJob = state.jobs.find((job) => job.id === state.jobDetails.selectedId);
        if (currentJob) {
          state.jobDetails.job = currentJob;
          if (jobDetailDialog.open) renderJobDetail();
        } else if (jobDetailDialog.open) {
          state.jobDetails.job = null;
          state.jobDetails.error = "This job is no longer in the workspace.";
          renderJobDetail();
        }
      }
      if (state.route === "logs" && state.logs.jobId) {
        try {
          state.logs.data = await request(`/api/v1/jobs/${state.logs.jobId}/logs`);
          state.logs.error = null;
        } catch (error) {
          state.logs.data = null;
          state.logs.error = error.message;
        }
      }
      state.loaded = true;
      document.getElementById("app-version").textContent = state.config.version || "—";
      updateConnection(true);
      const shouldRender = options.forceRender || state.renderAfterLoad || !isInteractiveEditing();
      state.renderAfterLoad = false;
      if (shouldRender) render();
      else {
        updateSchedulerPill();
        refreshLiveTimes();
      }
    } catch (error) {
      state.error = error.message;
      updateConnection(false);
      if (state.loaded) showToast(state.error, true);
      else renderError(state.error);
    } finally {
      state.loading = false;
    }
  }

  function render() {
    const focusSnapshot = captureFocus();
    const route = ["overview", "jobs", "queue", "logs", "configuration"].includes(state.route) ? state.route : "overview";
    state.route = route;
    app.dataset.view = route;
    document.querySelectorAll("[data-route]").forEach((item) => item.classList.toggle("active", item.dataset.route === route));
    document.getElementById("breadcrumb-current").textContent = routeTitle(route);
    updateSchedulerPill();
    if (route === "overview") renderOverview();
    else if (route === "jobs") renderJobs();
    else if (route === "queue") renderQueue();
    else if (route === "logs") renderLogs();
    else if (route === "configuration") renderConfiguration();
    else renderNotice(route);
    restoreFocus(focusSnapshot);
  }

  function renderLoading() {
    app.innerHTML = '<div class="page-loading"><span class="spinner"></span><span>Loading workspace…</span></div>';
  }

  function renderError(message) {
    app.innerHTML = `<div class="page-heading"><div><div class="eyebrow">Workspace unavailable</div><h1>Couldn’t load Stoker</h1><p>${escapeHtml(message)}</p></div><button class="button primary" id="retry-button">Try again</button></div>`;
    document.getElementById("retry-button").addEventListener("click", loadData);
  }

  function pageInfo(items, requestedPage, pageSize) {
    const totalPages = Math.max(1, Math.ceil(items.length / pageSize));
    const page = Math.min(Math.max(Number(requestedPage) || 1, 1), totalPages);
    const start = (page - 1) * pageSize;
    return { page, totalPages, totalItems: items.length, start, end: Math.min(start + pageSize, items.length), items: items.slice(start, start + pageSize) };
  }

  function paginationMarkup(kind, info, label) {
    if (info.totalPages <= 1 && kind !== "jobs") return "";
    const from = info.totalItems ? info.start + 1 : 0;
    const to = info.end;
    return `<nav class="list-pagination" aria-label="${escapeAttribute(label)} pagination"><span>Showing ${from}–${to} of ${info.totalItems}</span><div class="list-pagination-controls"><button class="button small secondary" type="button" data-page-kind="${kind}" data-page-number="${info.page - 1}" ${info.page === 1 ? "disabled" : ""}>Previous</button><span>Page ${info.page} of ${info.totalPages}</span><button class="button small secondary" type="button" data-page-kind="${kind}" data-page-number="${info.page + 1}" ${info.page === info.totalPages ? "disabled" : ""}>Next</button></div></nav>`;
  }

  function bindPagination(kind, renderPage) {
    document.querySelectorAll(`[data-page-kind="${kind}"]`).forEach((button) => {
      button.addEventListener("click", () => {
        state.pagination[kind] = Number(button.dataset.pageNumber);
        renderPage();
      });
    });
  }

  function renderOverview() {
    const counts = state.status.counts;
    const active = state.jobs.find((job) => ["STARTING", "RUNNING", "CANCELLING"].includes(job.state));
    const queue = state.queue.jobs.slice(0, 5);
    app.innerHTML = `
      <div class="page-heading">
        <div><div class="eyebrow">Queue control center</div><h1>See what’s running and what’s next.</h1><p>Keep long-running work moving with a clear view of what is running, waiting, and ready to ship.</p></div>
        <button class="button secondary" id="overview-refresh">Refresh workspace <span aria-hidden="true">↻</span></button>
      </div>
      <div class="metrics-grid">
        ${metric("Running", counts.active, active ? active.name : "No active job", "accent-ember")}
        ${metric("Queued", counts.queued, state.queue.locked ? "Queue is locked" : "Ready to run", "accent-cyan")}
        ${metric("Drafts", counts.draft, "Awaiting review", "")}
        ${metric("Succeeded", counts.succeeded, "Completed jobs", "accent-green")}
        ${metric("Failed / lost", counts.failed, "Needs attention", "")}
      </div>
      <div class="content-grid">
        <section class="panel">
          <div class="panel-header"><div class="panel-title"><div><h2>Active job</h2><p>What the scheduler is working on right now</p></div></div><a class="panel-link" href="#jobs">View all jobs →</a></div>
          <div class="active-job">${activeJobMarkup(active)}</div>
        </section>
        <section class="panel">
          <div class="panel-header"><div class="panel-title"><div><h2>Up next</h2><p>Queue execution order</p></div></div><a class="panel-link" href="#queue">Manage queue →</a></div>
          <div class="queue-list">${queueMarkup(queue)}</div>
        </section>
        <section class="panel">
          <div class="panel-header"><div class="panel-title"><div><h2>Recent activity</h2><p>Latest persisted job events</p></div></div><a class="panel-link" href="#jobs">Open jobs →</a></div>
          <div class="activity-list">${activityMarkup(state.jobs)}</div>
        </section>
        <section class="panel">
          <div class="notice-panel"><div class="notice-icon">⌘</div><div><div class="section-kicker">Terminal workflow</div><h2>${state.status.scheduler.running ? "Scheduler is online" : "Scheduler is stopped"}</h2><p>${state.status.scheduler.running ? "Newly committed jobs can be claimed in queue order." : "Run <code>stoker start</code> in the terminal to process queued work. The browser UI remains available for inspection."}</p></div></div>
        </section>
      </div>`;
    document.getElementById("overview-refresh").addEventListener("click", loadData);
  }

  function renderJobs(pageSize = JOBS_PAGE_SIZE) {
    const focusSnapshot = captureFocus();
    const filtered = state.jobs.filter((job) => {
      const search = state.filters.search.toLowerCase();
      const matchesSearch = !search || [job.name, job.user, job.id, job.cwd].some((value) => String(value || "").toLowerCase().includes(search));
      return matchesSearch && (!state.filters.user || job.user === state.filters.user) && (!state.filters.state || job.state === state.filters.state);
    });
    const users = [...new Set(state.jobs.map((job) => job.user))].sort();
    const terminalCount = state.jobs.filter((job) => isTerminalState(job.state)).length;
    const requestedPage = state.pagination.jobs;
    const jobsPage = pageInfo(filtered, requestedPage, pageSize);
    state.pagination.jobs = jobsPage.page;
    app.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">Workspace / Jobs</div><h1>All jobs</h1><p>Search every submission, inspect its working directory, and keep an eye on its lifecycle.</p></div><div class="page-actions"><button class="button primary" id="jobs-new" type="button">＋ New job</button><button class="button secondary" id="jobs-refresh">Refresh jobs <span aria-hidden="true">↻</span></button><button class="button danger" id="jobs-clean" type="button" ${terminalCount ? "" : "disabled"}>Clean job history${terminalCount ? ` (${terminalCount})` : ""}</button></div></div>
      <div class="toolbar"><input class="search-input" id="job-search" type="search" autocomplete="off" spellcheck="false" placeholder="Search job name, owner, ID, or path" value="${escapeAttribute(state.filters.search)}" aria-label="Search jobs"><div class="filter-group"><select class="select-input" id="user-filter" aria-label="Filter by owner"><option value="">All owners</option>${users.map((user) => `<option value="${escapeAttribute(user)}" ${user === state.filters.user ? "selected" : ""}>${escapeHtml(user)}</option>`).join("")}</select><select class="select-input" id="state-filter" aria-label="Filter by state"><option value="">All states</option>${["DRAFT", "QUEUED", "STARTING", "RUNNING", "CANCELLING", "SUCCEEDED", "FAILED", "CANCELLED", "LOST"].map((value) => `<option value="${value}" ${value === state.filters.state ? "selected" : ""}>${value}</option>`).join("")}</select></div></div>
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>${filtered.length} visible job${filtered.length === 1 ? "" : "s"}</h2><p>Server state · ${liveTime(new Date())}</p></div></div></div><div class="table-wrap"><table class="data-table jobs-table"><thead><tr><th>Job name</th><th>Owner</th><th>Path</th><th>State</th><th>Queue</th><th>Created</th></tr></thead><tbody>${filtered.length ? jobsPage.items.map(jobRow).join("") : emptyTableRow("No jobs match these filters.", 6)}</tbody></table></div>${paginationMarkup("jobs", jobsPage, "Jobs")}</section>`;
    // Measure the actual header, controls, row and pagination heights, including
    // wrapping and horizontal scrollbars, before choosing how many rows fit.
    const rows = [...app.querySelectorAll(".jobs-table .job-row")];
    if (rows.length && pageSize === JOBS_PAGE_SIZE) {
      const rowHeight = Math.max(...rows.map((row) => row.getBoundingClientRect().height));
      const panel = app.querySelector(".panel");
      const spareHeight = listViewportBottom() - panel.getBoundingClientRect().bottom;
      const fittedSize = Math.max(1, Math.min(JOBS_PAGE_SIZE, Math.floor((rows.length * rowHeight + spareHeight) / rowHeight)));
      if (fittedSize < JOBS_PAGE_SIZE) {
        state.pagination.jobs = requestedPage;
        renderJobs(fittedSize);
        restoreFocus(focusSnapshot);
        return;
      }
    }
    document.getElementById("jobs-refresh").addEventListener("click", loadData);
    document.getElementById("jobs-new").addEventListener("click", openJobDialog);
    document.getElementById("jobs-clean").addEventListener("click", async () => {
      const confirmed = await openConfirmation({
        kicker: "Workspace cleanup",
        title: "Clean job history?",
        message: "This permanently removes SUCCEEDED, FAILED, CANCELLED, and LOST jobs with their logs. Draft, queued, and active jobs will stay.",
        acceptLabel: "Clean history",
      });
      if (confirmed) {
        mutate("/api/v1/clean", "POST", null, (response) => {
          const removed = Number(response.removed || 0);
          return `Cleaned ${removed} job${removed === 1 ? "" : "s"} from history.`;
        });
      }
    });
    document.getElementById("job-search").addEventListener("input", (event) => { state.filters.search = event.target.value; state.pagination.jobs = 1; renderJobs(); });
    document.getElementById("user-filter").addEventListener("change", (event) => { state.filters.user = event.target.value; state.pagination.jobs = 1; renderJobs(); });
    document.getElementById("state-filter").addEventListener("change", (event) => { state.filters.state = event.target.value; state.pagination.jobs = 1; renderJobs(); });
    bindPagination("jobs", renderJobs);
    document.querySelectorAll("[data-job-open]").forEach((row) => {
      const open = () => openJobDetail(row.dataset.jobOpen);
      row.addEventListener("click", open);
      row.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          open();
        }
      });
    });
    restoreFocus(focusSnapshot);
  }

  async function openJobDetail(id) {
    state.jobDetails.selectedId = id;
    state.jobDetails.job = null;
    state.jobDetails.workingDirectoryStatus = "";
    state.jobDetails.displayTimezone = "";
    state.jobDetails.loading = true;
    state.jobDetails.action = "";
    state.jobDetails.error = "";
    const requestId = ++state.jobDetails.requestId;
    if (!jobDetailDialog.open) jobDetailDialog.showModal();
    renderJobDetail();
    try {
      const result = await request(`/api/v1/jobs/${encodeURIComponent(id)}`);
      if (requestId !== state.jobDetails.requestId || state.jobDetails.selectedId !== id) return;
      state.jobDetails.job = result.job;
      state.jobDetails.workingDirectoryStatus = result.working_directory_status || "";
      state.jobDetails.displayTimezone = result.display_timezone || "";
      state.jobDetails.error = "";
    } catch (error) {
      if (requestId !== state.jobDetails.requestId || state.jobDetails.selectedId !== id) return;
      state.jobDetails.error = error.message;
    } finally {
      if (requestId === state.jobDetails.requestId) state.jobDetails.loading = false;
      if (state.jobDetails.selectedId === id) renderJobDetail();
    }
  }

  function closeJobDetail() {
    state.jobDetails.selectedId = "";
    state.jobDetails.job = null;
    state.jobDetails.action = "";
    state.jobDetails.error = "";
    state.jobDetails.requestId += 1;
    if (jobDetailDialog.open) jobDetailDialog.close();
  }

  function renderJobDetail() {
    if (!jobDetailDialog.open && !state.jobDetails.selectedId) return;
    const detail = state.jobDetails;
    if (detail.loading) {
      jobDetailContent.innerHTML = `<div class="job-detail-card"><div class="job-detail-heading"><div><div class="dialog-kicker">Workspace / Jobs</div><h2 id="job-detail-title">Job details</h2></div><button class="dialog-close" id="job-detail-close" type="button" aria-label="Close job details">×</button></div><div class="page-loading"><span class="spinner"></span><span>Loading job details…</span></div></div>`;
      document.getElementById("job-detail-close").addEventListener("click", closeJobDetail);
      return;
    }
    if (detail.error || !detail.job) {
      jobDetailContent.innerHTML = `<div class="job-detail-card"><div class="job-detail-heading"><div><div class="dialog-kicker">Workspace / Jobs</div><h2 id="job-detail-title">Job details</h2></div><button class="dialog-close" id="job-detail-close" type="button" aria-label="Close job details">×</button></div><div class="job-detail-error"><strong>Couldn’t load this job</strong><p>${escapeHtml(detail.error || "The job is no longer available.")}</p><div class="dialog-actions"><button class="button secondary" id="job-detail-retry" type="button">Try again</button></div></div></div>`;
      document.getElementById("job-detail-close").addEventListener("click", closeJobDetail);
      document.getElementById("job-detail-retry").addEventListener("click", () => openJobDetail(detail.selectedId));
      return;
    }
    const job = detail.job;
    const command = job.command_line || (Array.isArray(job.command) ? job.command.join(" ") : job.command) || "—";
    const actionButtons = jobActionMarkup(job, detail.action);
    const directoryStatus = detail.workingDirectoryStatus || workingDirectoryStatus(job.state);
    jobDetailContent.innerHTML = `
      <div class="job-detail-card">
        <div class="job-detail-heading"><div><div class="dialog-kicker">Workspace / Jobs</div><h2 id="job-detail-title">${escapeHtml(job.name)}</h2><p class="job-detail-id mono">${escapeHtml(job.id)}</p></div><button class="dialog-close" id="job-detail-close" type="button" aria-label="Close job details">×</button></div>
        <div class="job-detail-status"><div><span class="section-kicker">Current state</span><div class="job-detail-state">${stateBadge(job.state)}</div></div><div class="job-detail-status-meta"><span class="job-detail-directory-status">${escapeHtml(directoryStatus)}</span>${detail.displayTimezone ? `<span class="job-detail-timezone">Times shown in ${escapeHtml(detail.displayTimezone)}</span>` : ""}</div></div>
        ${detail.error ? `<div class="form-feedback invalid" aria-live="polite">${escapeHtml(detail.error)}</div>` : ""}
        <section class="job-detail-section"><div class="section-kicker">Job overview</div><div class="job-detail-grid"><div><span class="detail-label">Owner</span><strong>${escapeHtml(job.user)}</strong></div><div><span class="detail-label">Queue order</span><strong>${job.queue_order == null ? "—" : escapeHtml(job.queue_order)}</strong></div><div class="job-detail-wide"><span class="detail-label">Working directory</span><code>${escapeHtml(job.cwd)}</code></div><div class="job-detail-wide"><span class="detail-label">Command</span><pre class="job-command">${escapeHtml(command)}</pre></div></div></section>
        <section class="job-detail-section"><div class="section-kicker">Timeline</div><div class="job-timeline"><div><span>Created</span>${liveTime(job.created_at)}</div><div><span>Committed</span>${liveTime(job.committed_at)}</div><div><span>Started</span>${liveTime(job.started_at)}</div><div><span>Finished</span>${liveTime(job.finished_at)}</div></div></section>
        <section class="job-detail-section"><div class="section-kicker">Result</div><div class="job-detail-grid"><div><span class="detail-label">Process ID</span><strong>${job.pid == null ? "—" : escapeHtml(job.pid)}</strong></div><div><span class="detail-label">Exit code</span><strong>${job.exit_code == null ? "—" : escapeHtml(job.exit_code)}</strong></div>${job.failure_detail ? `<div class="job-detail-wide detail-failure"><span class="detail-label">Failure detail</span><p>${escapeHtml(job.failure_detail)}</p></div>` : ""}</div></section>
        <div class="job-detail-actions"><button class="button secondary" id="job-detail-logs" type="button">View logs</button><span class="job-action-spacer"></span>${actionButtons}</div>
      </div>`;
    document.getElementById("job-detail-close").addEventListener("click", closeJobDetail);
    document.getElementById("job-detail-logs").addEventListener("click", () => {
      state.logs.jobId = job.id;
      state.logs.data = null;
      state.logs.error = null;
      closeJobDetail();
      if (location.hash !== "#logs") location.hash = "#logs";
      else render();
      loadData({ forceRender: true });
    });
    const commit = document.getElementById("job-detail-commit");
    if (commit) commit.addEventListener("click", () => confirmJobAction("commit", job));
    const cancel = document.getElementById("job-detail-cancel");
    if (cancel) cancel.addEventListener("click", () => confirmJobAction("cancel", job));
  }

  function jobActionMarkup(job, action) {
    if (action) return `<button class="button primary" type="button" disabled><span class="spinner small-spinner"></span>${action === "commit" ? "Committing…" : "Cancelling…"}</button>`;
    if (job.state === "DRAFT") return '<button class="button danger" id="job-detail-cancel" type="button">Cancel job</button><button class="button primary" id="job-detail-commit" type="button">Commit job</button>';
    if (["QUEUED", "STARTING", "RUNNING"].includes(job.state)) return '<button class="button danger" id="job-detail-cancel" type="button">Cancel job</button>';
    if (job.state === "CANCELLING") return '<button class="button secondary" type="button" disabled>Cancelling…</button>';
    return '<span class="job-action-note">This job has finished and no actions are available.</span>';
  }

  async function confirmJobAction(action, job) {
    const commit = action === "commit";
    const confirmed = await openConfirmation({
      kicker: `Job / ${action}`,
      title: `${commit ? "Commit" : "Cancel"} ${job.name}?`,
      message: commit ? "This moves the DRAFT into the scheduler queue and makes it eligible to run." : "This asks the scheduler to stop the job or marks a DRAFT as cancelled.",
      acceptLabel: commit ? "Commit job" : "Cancel job",
      destructive: !commit,
    });
    if (confirmed) runJobAction(action, job.id);
  }

  async function runJobAction(action, id) {
    if (state.jobDetails.action) return;
    state.jobDetails.action = action;
    state.jobDetails.error = "";
    renderJobDetail();
    try {
      const response = await request(`/api/v1/jobs/${encodeURIComponent(id)}/${action}`, { method: "POST" });
      state.jobDetails.job = response.job;
      showToast(action === "commit" ? "Job committed to the queue." : "Cancel request sent.");
      await loadData({ forceRender: true });
      state.jobDetails.action = "";
      if (state.jobDetails.selectedId === id) renderJobDetail();
    } catch (error) {
      state.jobDetails.action = "";
      state.jobDetails.error = error.message;
      showToast(error.message, true);
      renderJobDetail();
    }
  }

  function workingDirectoryStatus(value) {
    if (["DRAFT", "QUEUED"].includes(value)) return "planned";
    if (["STARTING", "RUNNING", "CANCELLING"].includes(value)) return "active";
    return "source directory retained";
  }

  async function openJobDialog() {
    state.jobDialogOpen = true;
    state.jobDialogView = "form";
    state.filesystem.error = "";
    state.filesystem.loading = false;
    if (!state.filesystem.roots) {
      try {
        const roots = await request("/api/v1/fs/roots");
        state.filesystem.roots = roots;
        if (!state.jobDraft.cwd) state.jobDraft.cwd = roots.default_path || "";
      } catch (error) {
        state.filesystem.error = error.message;
      }
    }
    if (!jobDialog.open) jobDialog.showModal();
    renderJobDialog();
    jobDialog.focus({ preventScroll: true });
  }

  function closeJobDialog() {
    state.jobDialogOpen = false;
    state.jobDialogView = "form";
    if (jobDialog.open) jobDialog.close();
  }

  function renderJobDialog() {
    if (!state.jobDialogOpen) return;
    if (state.jobDialogView === "browser") renderFilesystemBrowser();
    else renderJobForm();
  }

  function renderJobForm() {
    const draft = state.jobDraft;
    const users = [...new Set(state.jobs.map((job) => job.user).filter(Boolean))].sort();
    jobDialogContent.innerHTML = `
      <div class="dialog-card job-card">
        <div class="dialog-kicker">Workspace / Jobs</div>
        <div class="job-dialog-heading"><div><h2 id="job-dialog-title">New job</h2><p>Create a draft job. Commit it when you're ready to run.</p></div><button class="dialog-close" id="job-dialog-close" type="button" aria-label="Close new job">×</button></div>
        <form id="new-job-form" novalidate>
          <div class="job-form-grid">
            <div class="job-field"><label for="job-user">Owner</label><div class="job-user-picker"><input class="text-input" id="job-user" name="user" value="${escapeAttribute(draft.user)}" autocomplete="off" autocapitalize="off" aria-autocomplete="list" aria-controls="job-user-suggestions" aria-expanded="false" required><div class="job-suggestions" id="job-user-suggestions" role="listbox"></div></div><small class="form-help">A logical label for this job.</small></div>
            <div class="job-field"><label for="job-name">Job name</label><input class="text-input" id="job-name" name="name" value="${escapeAttribute(draft.name)}" autocomplete="off" required></div>
          </div>
          <div class="job-field"><label for="job-cwd">Working directory</label><div class="path-input-row"><input class="text-input mono-input" id="job-cwd" name="cwd" value="${escapeAttribute(draft.cwd)}" placeholder="Choose a folder on the Stoker host" autocomplete="off" autocapitalize="off" spellcheck="false" required><button class="button secondary" id="job-browse" type="button">Browse</button></div><small class="form-help">Folders are read from the Stoker host.</small></div>
          <div class="job-field"><label for="job-command">Command</label><textarea class="command-input" id="job-command" name="command" rows="4" placeholder="cargo build --release" required>${escapeHtml(draft.command)}</textarea><small class="form-help">Enter the command only; Stoker will keep the same quoting rules as <code>stoker add --cmd</code>.</small></div>
          <div class="form-feedback ${state.filesystem.error ? "invalid" : ""}" id="job-form-feedback" aria-live="polite">${escapeHtml(state.filesystem.error)}</div>
          <div class="dialog-actions"><button class="button secondary" id="job-cancel" type="button">Cancel</button><button class="button primary" id="job-create" type="submit">Create draft</button></div>
        </form>
      </div>`;
    const form = document.getElementById("new-job-form");
    ["user", "name", "cwd", "command"].forEach((field) => {
      document.getElementById(`job-${field}`).addEventListener("input", (event) => { state.jobDraft[field] = event.target.value; if (field === "user") renderUserSuggestions(); });
    });
    const userInput = document.getElementById("job-user");
    const userSuggestions = document.getElementById("job-user-suggestions");
    const renderUserSuggestions = () => {
      const query = userInput.value.trim().toLowerCase();
      const matchingUsers = users.filter((user) => !query || user.toLowerCase().includes(query));
      const matches = matchingUsers.slice(0, MAX_OWNER_SUGGESTIONS);
      const label = matchingUsers.length > MAX_OWNER_SUGGESTIONS
        ? `Known owners · Showing ${MAX_OWNER_SUGGESTIONS} of ${matchingUsers.length}`
        : `Known owners · ${matchingUsers.length}`;
      userSuggestions.innerHTML = matches.length ? `<div class="job-suggestions-label">${escapeHtml(label)}</div>${matches.map((user) => `<button class="job-suggestion" type="button" role="option" data-job-user="${escapeAttribute(user)}"><span class="suggestion-avatar">${escapeHtml(user.slice(0, 1).toUpperCase())}</span><span>${escapeHtml(user)}</span></button>`).join("")}` : "";
      const visible = matches.length > 0 && document.activeElement === userInput;
      userSuggestions.classList.toggle("visible", visible);
      userInput.setAttribute("aria-expanded", String(visible));
      userSuggestions.querySelectorAll("[data-job-user]").forEach((button) => {
        button.addEventListener("mousedown", (event) => event.preventDefault());
        button.addEventListener("click", () => {
          state.jobDraft.user = button.dataset.jobUser;
          userInput.value = state.jobDraft.user;
          userSuggestions.classList.remove("visible");
          userInput.setAttribute("aria-expanded", "false");
          userInput.focus({ preventScroll: true });
        });
      });
    };
    userInput.addEventListener("focus", renderUserSuggestions);
    userInput.addEventListener("blur", () => setTimeout(() => { userSuggestions.classList.remove("visible"); userInput.setAttribute("aria-expanded", "false"); }, 120));
    document.getElementById("job-browse").addEventListener("click", () => {
      state.jobDialogView = "browser";
      renderFilesystemBrowser();
      loadDirectories(state.jobDraft.cwd);
    });
    document.getElementById("job-cancel").addEventListener("click", closeJobDialog);
    document.getElementById("job-dialog-close").addEventListener("click", closeJobDialog);
    form.addEventListener("submit", submitNewJob);
  }

  async function submitNewJob(event) {
    event.preventDefault();
    const draft = state.jobDraft;
    const feedback = document.getElementById("job-form-feedback");
    const button = document.getElementById("job-create");
    if (!draft.user.trim() || !draft.name.trim() || !draft.cwd.trim() || !draft.command.trim()) {
      feedback.className = "form-feedback invalid";
      feedback.textContent = "Owner, job name, working directory, and command are required.";
      return;
    }
    button.disabled = true;
    button.textContent = "Creating…";
    feedback.className = "form-feedback";
    feedback.textContent = "Saving DRAFT to the Stoker host…";
    try {
      await request("/api/v1/jobs", { method: "POST", body: JSON.stringify(draft) });
      closeJobDialog();
      state.jobDraft = { user: "", name: "", cwd: "", command: "" };
      showToast("Created draft job.");
      await loadData({ forceRender: true });
    } catch (error) {
      button.disabled = false;
      button.textContent = "Create draft";
      feedback.className = "form-feedback invalid";
      feedback.textContent = error.message;
    }
  }

  function renderFilesystemBrowser() {
    const fsState = state.filesystem;
    const roots = fsState.roots && fsState.roots.locations ? fsState.roots.locations : [];
    const current = fsState.current;
    const tabs = roots.map((location, index) => `<button class="fs-root-button" type="button" data-fs-root="${index}">${escapeHtml(location.label)}</button>`).join("");
    const rows = fsState.error
      ? `<div class="fs-empty fs-empty-error"><span>!</span><strong>Folder unavailable</strong><small>${escapeHtml(fsState.error)}</small></div>`
      : current && current.directories && current.directories.length
        ? current.directories.map((directory) => `<button class="fs-directory-row" type="button" data-fs-directory="${escapeAttribute(directory.path)}"><span class="fs-folder-glyph">▰</span><span>${escapeHtml(directory.name)}</span><span class="fs-row-arrow">›</span></button>`).join("")
        : `<div class="fs-empty"><span>○</span><strong>${fsState.loading ? "Loading folders…" : "No subfolders here"}</strong><small>${fsState.loading ? "Reading one level from the Stoker host." : "This folder is still available to select."}</small></div>`;
    const feedback = fsState.error ? `<div class="form-feedback invalid" aria-live="polite">${escapeHtml(fsState.error)} <button class="button ghost small" id="fs-retry" type="button">Retry</button></div>` : current && current.truncated ? '<div class="form-feedback">Some folders are hidden because this directory is very large. Enter a full path to browse further.</div>' : "";
    jobDialogContent.innerHTML = `
      <div class="dialog-card job-card filesystem-card">
        <div class="dialog-kicker">Working directory</div>
        <div class="job-dialog-heading"><div><h2 id="job-dialog-title">Choose working directory</h2><p>Folders on the Stoker host</p></div><button class="dialog-close" id="fs-close" type="button" aria-label="Close folder chooser">×</button></div>
        <div class="fs-root-tabs" role="tablist" aria-label="Folder shortcuts">${tabs || '<span class="fs-no-roots">No shortcuts available</span>'}</div>
        <div class="fs-path-row"><input class="text-input mono-input" id="fs-path-input" value="${escapeAttribute(fsState.inputPath || (current ? current.path : state.jobDraft.cwd))}" aria-label="Folder path" autocomplete="off" autocapitalize="off" spellcheck="false"><button class="button secondary" id="fs-go" type="button">Go</button></div>
        <div class="fs-breadcrumb" title="${escapeAttribute(current ? current.path : "")}">${escapeHtml(current ? current.path : "Choose a path to browse")}</div>
        <div class="fs-browser-toolbar"><button class="button small secondary" id="fs-up" type="button" ${!current || !current.parent ? "disabled" : ""}>↑ Up</button><span>${current ? `${current.directories.length} folder${current.directories.length === 1 ? "" : "s"}` : ""}</span></div>
        <div class="fs-directory-list" aria-live="polite">${rows}</div>${feedback}
        <div class="fs-selected"><span>Selected</span><code>${escapeHtml(current ? current.path : "—")}</code></div>
        <div class="dialog-actions"><button class="button secondary" id="fs-back" type="button">Back</button><button class="button primary" id="fs-use" type="button" ${!current || fsState.loading ? "disabled" : ""}>Use this folder</button></div>
      </div>`;
    document.getElementById("fs-close").addEventListener("click", closeJobDialog);
    document.getElementById("fs-back").addEventListener("click", () => { state.jobDialogView = "form"; renderJobDialog(); });
    document.getElementById("fs-use").addEventListener("click", () => { if (current) { state.jobDraft.cwd = current.path; state.jobDialogView = "form"; renderJobDialog(); } });
    document.getElementById("fs-go").addEventListener("click", () => loadDirectories(document.getElementById("fs-path-input").value));
    document.getElementById("fs-path-input").addEventListener("input", (event) => { fsState.inputPath = event.target.value; });
    document.getElementById("fs-path-input").addEventListener("keydown", (event) => { if (event.key === "Enter") { event.preventDefault(); loadDirectories(event.target.value); } });
    document.getElementById("fs-up").addEventListener("click", () => { if (current && current.parent) loadDirectories(current.parent); });
    document.querySelectorAll("[data-fs-root]").forEach((button) => button.addEventListener("click", () => loadDirectories(roots[Number(button.dataset.fsRoot)].path)));
    document.querySelectorAll("[data-fs-directory]").forEach((button) => {
      const path = button.dataset.fsDirectory;
      button.addEventListener("pointerenter", () => prefetchDirectory(path));
      button.addEventListener("pointerdown", () => prefetchDirectory(path));
      button.addEventListener("focus", () => prefetchDirectory(path));
      button.addEventListener("click", () => loadDirectories(path));
    });
    const retry = document.getElementById("fs-retry");
    if (retry) retry.addEventListener("click", () => loadDirectories(fsState.inputPath || (current && current.path)));
  }

  function cachedDirectory(path) {
    return state.filesystem.directoryCache.get(path) || null;
  }

  function cacheDirectory(path, result) {
    const directoryCache = state.filesystem.directoryCache;
    directoryCache.delete(path);
    directoryCache.set(path, { result, fetchedAt: Date.now() });
    while (directoryCache.size > DIRECTORY_CACHE_LIMIT) directoryCache.delete(directoryCache.keys().next().value);
  }

  function requestDirectories(path) {
    const fsState = state.filesystem;
    const existing = fsState.directoryRequests.get(path);
    if (existing) return existing;
    const promise = request(`/api/v1/fs/directories?${new URLSearchParams({ path })}`)
      .then((result) => {
        cacheDirectory(path, result);
        return result;
      })
      .finally(() => fsState.directoryRequests.delete(path));
    fsState.directoryRequests.set(path, promise);
    return promise;
  }

  function prefetchDirectory(path) {
    if (!path || !state.jobDialogOpen || state.jobDialogView !== "browser") return;
    const cached = cachedDirectory(path);
    if (cached && Date.now() - cached.fetchedAt < DIRECTORY_CACHE_TTL_MS) return;
    requestDirectories(path).catch(() => {});
  }

  async function loadDirectories(path) {
    if (!path || state.jobDialogView !== "browser") return;
    const fsState = state.filesystem;
    const requestId = ++fsState.requestId;
    const cached = cachedDirectory(path);
    const hasCached = Boolean(cached);
    fsState.loading = !hasCached;
    fsState.error = "";
    fsState.inputPath = path;
    fsState.current = cached ? cached.result : null;
    renderFilesystemBrowser();
    try {
      const result = await requestDirectories(path);
      if (requestId !== fsState.requestId || !state.jobDialogOpen) return;
      fsState.current = result;
      fsState.inputPath = result.path;
      fsState.error = "";
    } catch (error) {
      if (requestId !== fsState.requestId || !state.jobDialogOpen) return;
      fsState.error = error.message;
    } finally {
      if (requestId === fsState.requestId) fsState.loading = false;
      if (state.jobDialogOpen && state.jobDialogView === "browser") renderFilesystemBrowser();
    }
  }

  function renderQueue() {
    const previousTable = app.querySelector(".queue-table");
    const scrollTop = previousTable?.scrollTop || 0;
    const scrollLeft = previousTable?.scrollLeft || 0;
    const jobs = state.queue.jobs;
    const locked = Boolean(state.queue.locked);
    app.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">Workspace / Queue</div><h1>Execution queue</h1><p>Lock the global queue before changing its order. Every move is checked against the server’s latest state.</p></div><div class="page-actions"><button class="button secondary" id="queue-refresh">Refresh queue <span aria-hidden="true">↻</span></button><button class="button primary" id="queue-lock" ${locked ? "disabled" : ""}>Lock queue</button><button class="button secondary" id="queue-unlock" ${locked ? "" : "disabled"}>Unlock queue</button></div></div>
      ${locked ? '<div class="queue-lock-banner"><div><strong>Queue is locked</strong><small>This global lock is visible to every connected client. Use the arrows below to adjust order.</small></div><span aria-hidden="true">🔒</span></div>' : '<div class="queue-unlock-banner"><div><strong>Queue is unlocked</strong><small>Lock the queue to enable reorder controls and prevent the scheduler from claiming work during edits.</small></div><span aria-hidden="true">↕</span></div>'}
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>${jobs.length} queued job${jobs.length === 1 ? "" : "s"}</h2><p>Ordered by queue position · server state</p></div></div></div><div class="queue-table"><table class="data-table queue-order-table"><thead><tr><th>#</th><th>Job name</th><th>Owner</th><th>Path</th><th>State</th><th>Move</th></tr></thead><tbody>${jobs.length ? jobs.map((job, index) => `<tr><td class="mono">${index + 1}</td><td><div class="job-name"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(shortId(job.id))}</small></div></td><td>${escapeHtml(job.user)}</td><td class="path-cell" title="${escapeAttribute(job.cwd)}">${escapeHtml(job.cwd)}</td><td>${stateBadge(job.state)}</td><td><div class="move-controls"><button class="move-button" type="button" data-queue-move="${escapeAttribute(job.id)}" data-target-order="${index}" aria-label="Move ${escapeAttribute(job.name)} up" ${!locked || index === 0 ? "disabled" : ""}>↑</button><button class="move-button" type="button" data-queue-move="${escapeAttribute(job.id)}" data-target-order="${index + 2}" aria-label="Move ${escapeAttribute(job.name)} down" ${!locked || index === jobs.length - 1 ? "disabled" : ""}>↓</button></div></td></tr>`).join("") : emptyTableRow("The queue is clear. Commit a DRAFT job from the Jobs view or CLI.", 6)}</tbody></table></div></section>`;
    const table = app.querySelector(".queue-table");
    // Use the remaining viewport instead of assuming a fixed heading height.
    table.style.maxHeight = `${Math.max(100, listViewportBottom() - table.getBoundingClientRect().top - 1)}px`;
    table.scrollTop = scrollTop;
    table.scrollLeft = scrollLeft;
    document.getElementById("queue-refresh").addEventListener("click", loadData);
    document.getElementById("queue-lock").addEventListener("click", () => mutate("/api/v1/queue/lock", "POST"));
    document.getElementById("queue-unlock").addEventListener("click", () => mutate("/api/v1/queue/unlock", "POST"));
    document.querySelectorAll("[data-queue-move]").forEach((button) => {
      button.addEventListener("click", () => mutate(`/api/v1/queue/${button.dataset.queueMove}/move`, "POST", { target_order: Number(button.dataset.targetOrder) }));
    });
  }

  function renderLogs() {
    const focusSnapshot = captureFocus();
    const availableJobs = state.jobs;
    if (state.logs.jobId && !availableJobs.some((job) => job.id === state.logs.jobId)) {
      state.logs.jobId = "";
      state.logs.data = null;
      state.logs.error = null;
    }
    const selectedJob = availableJobs.find((job) => job.id === state.logs.jobId);
    const logQuery = state.logs.jobSearch.trim().toLowerCase();
    const matchingJobs = availableJobs.filter((job) => !logQuery || [job.name, job.user, job.id, job.cwd].some((value) => String(value || "").toLowerCase().includes(logQuery)));
    const logJobs = selectedJob && !matchingJobs.some((job) => job.id === selectedJob.id) ? [selectedJob, ...matchingJobs] : matchingJobs;
    const data = state.logs.data;
    const stream = state.logs.stream === "stderr" ? "stderr" : "stdout";
    const streamAvailable = data ? Boolean(data[`${stream}_available`]) : false;
    const streamText = data ? data[stream] || "" : "";
    const streamTruncated = data ? Boolean(data[`${stream}_truncated`]) : false;
    const message = state.logs.error || (data && data.message) || (!availableJobs.length ? "No jobs are available yet. Add a job with the CLI first." : (!streamAvailable && selectedJob ? `No ${stream} log is available for this job yet.` : ""));
    const jobOptions = logJobs.length ? logJobs.map((job) => `<option value="${escapeAttribute(job.id)}" ${job.id === state.logs.jobId ? "selected" : ""}>${escapeHtml(job.name)} · ${escapeHtml(shortId(job.id))} · ${escapeHtml(job.state)}</option>`).join("") : '<option value="">No matching jobs</option>';
    const output = selectedJob && data && streamAvailable ? streamText || "(No output written yet.)" : "Select a job to inspect its output.";
    app.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">Workspace / Logs</div><h1>Job logs</h1><p>Choose a job, then switch between stdout and stderr. Logs are shown as plain text and are refreshed automatically every 2 seconds.</p></div><button class="button secondary" id="logs-refresh">Refresh logs <span aria-hidden="true">↻</span></button></div>
      <section class="panel logs-panel"><div class="logs-toolbar"><label for="log-job-select">Job</label><input class="search-input log-job-search" id="log-job-search" type="search" autocomplete="off" spellcheck="false" value="${escapeAttribute(state.logs.jobSearch)}" placeholder="Filter jobs" aria-label="Filter jobs"><select class="select-input log-job-select" id="log-job-select" aria-label="Choose a job"><option value="">Choose a job…</option>${jobOptions}</select><div class="log-tabs" role="tablist" aria-label="Log stream"><button class="log-tab ${stream === "stdout" ? "active" : ""}" type="button" data-log-stream="stdout" role="tab" aria-selected="${stream === "stdout"}">stdout</button><button class="log-tab ${stream === "stderr" ? "active" : ""}" type="button" data-log-stream="stderr" role="tab" aria-selected="${stream === "stderr"}">stderr</button></div></div>${selectedJob ? `<div class="log-context"><div><strong>${escapeHtml(selectedJob.name)}</strong><small>${escapeHtml(selectedJob.user)} · ${escapeHtml(selectedJob.cwd)}</small></div>${stateBadge(selectedJob.state)}</div>` : ""}${message ? `<div class="log-message"><span aria-hidden="true">ⓘ</span><span>${escapeHtml(message)}</span></div>` : ""}<pre class="log-output" aria-live="polite">${escapeHtml(output)}</pre>${streamTruncated ? `<div class="log-footnote">Showing the latest ${MAX_LOG_KB} KB of ${stream} output.</div>` : ""}</section>`;
    document.getElementById("logs-refresh").addEventListener("click", loadData);
    document.getElementById("log-job-search").addEventListener("input", (event) => {
      state.logs.jobSearch = event.target.value;
      renderLogs();
    });
    document.getElementById("log-job-select").addEventListener("change", (event) => {
      state.logs.jobId = event.target.value;
      state.logs.data = null;
      state.logs.error = null;
      renderLogs();
      loadData({ forceRender: true });
    });
    document.querySelectorAll("[data-log-stream]").forEach((button) => {
      button.addEventListener("click", () => {
        state.logs.stream = button.dataset.logStream;
        renderLogs();
      });
    });
    restoreFocus(focusSnapshot);
  }

  function renderConfiguration() {
    const settings = state.settings;
    if (!settings) {
      app.innerHTML = '<div class="page-loading"><span class="spinner"></span><span>Loading configuration…</span></div>';
      return;
    }
    const configuredTimezone = state.configurationDraft !== null ? state.configurationDraft : (settings.config.timezone || "");
    const timezone = settings.effective_timezone || { name: "—", source: "—" };
    const timezones = settings.timezones || [];
    const snapshots = settings.snapshots || [];
    const snapshotsPage = pageInfo(snapshots, state.pagination.snapshots, SNAPSHOTS_PAGE_SIZE);
    state.pagination.snapshots = snapshotsPage.page;
    const snapshotMarkup = snapshots.length ? snapshotsPage.items.map((snapshot) => `
      <div class="snapshot-row">
        <div class="snapshot-copy"><strong>${snapshot.created_at ? liveTime(snapshot.created_at) : "Invalid snapshot"}</strong><small>${escapeHtml(snapshot.reason || snapshot.error || snapshot.path)}</small></div>
        <div class="snapshot-actions">${snapshot.valid ? `<button class="button small secondary" type="button" data-restore-path="${escapeAttribute(snapshot.path)}">Restore</button>` : '<span class="state-badge state-failed">Invalid</span>'}</div>
      </div>`).join("") : '<div class="empty-state compact"><strong>No snapshots yet</strong><p>Save a manual snapshot before making a configuration change.</p></div>';
    app.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">Workspace / Configuration</div><h1>Workspace settings</h1><p>Manage the same timezone and configuration snapshots available from the CLI.</p></div><button class="button secondary" id="configuration-refresh">Refresh configuration <span aria-hidden="true">↻</span></button></div>
      <div class="config-layout">
        <section class="panel config-card"><div class="panel-header"><div class="panel-title"><div><h2>Timezone</h2><p>Used when displaying timestamps in the UI and CLI.</p></div></div></div><div class="config-summary"><div><span class="config-label">Effective timezone</span><strong>${escapeHtml(timezone.name)}</strong></div><div><span class="config-label">Source</span><span class="config-value">${escapeHtml(timezone.source)}</span></div><div><span class="config-label">Configured value</span><span class="config-value">${escapeHtml(configuredTimezone || "Not set · follows system")}</span></div></div><form class="config-form" id="timezone-form"><label for="timezone-input">Set IANA timezone</label><div class="timezone-picker"><input class="text-input" id="timezone-input" name="timezone" value="${escapeAttribute(configuredTimezone)}" placeholder="Type Tokyo, Taipei, UTC…" autocomplete="off" role="combobox" aria-autocomplete="list" aria-controls="timezone-suggestions" aria-expanded="false"><div class="timezone-suggestions" id="timezone-suggestions" role="listbox"></div></div><div id="timezone-feedback" class="form-feedback" aria-live="polite"></div><div class="form-row"><button class="button primary" id="timezone-set" type="submit" disabled>Set timezone</button><button class="button secondary" type="button" id="timezone-unset">Use system</button></div><small class="form-help">Type to search ${timezones.length} IANA timezones, then click a suggestion or press Enter.</small></form></section>
        <section class="panel config-card"><div class="panel-header"><div class="panel-title"><div><h2>Configuration snapshots</h2><p>Restore a known configuration without touching jobs.</p></div><button class="button small secondary" id="snapshot-create" type="button">Create snapshot</button></div></div><div class="snapshot-list">${snapshotMarkup}</div>${paginationMarkup("snapshots", snapshotsPage, "Configuration snapshots")}<div class="config-paths"><span>Config: <code>${escapeHtml(settings.config_path)}</code></span><span>Snapshots: <code>${escapeHtml(settings.snapshot_dir)}</code></span></div></section>
      </div>
      <section class="panel config-card"><div class="panel-header"><div class="panel-title"><div><h2>Current config</h2><p>Equivalent to <code>stoker config show</code>.</p></div></div></div><pre class="config-json">${escapeHtml(JSON.stringify(settings.config, null, 2))}</pre></section>`;
    document.getElementById("configuration-refresh").addEventListener("click", loadData);
    const timezoneInput = document.getElementById("timezone-input");
    const timezoneSuggestions = document.getElementById("timezone-suggestions");
    const timezoneFeedback = document.getElementById("timezone-feedback");
    const timezoneSet = document.getElementById("timezone-set");
    let timezoneSuggestionIndex = -1;
    const timezoneMatches = () => {
      const query = timezoneInput.value.trim().toLowerCase();
      if (!query) return [];
      return timezones.filter((value) => value.toLowerCase().includes(query)).sort((left, right) => {
        const leftStarts = left.toLowerCase().startsWith(query);
        const rightStarts = right.toLowerCase().startsWith(query);
        return Number(rightStarts) - Number(leftStarts) || left.localeCompare(right);
      }).slice(0, 8);
    };
    const updateTimezoneFeedback = () => {
      const value = timezoneInput.value.trim();
      const exact = timezones.includes(value);
      timezoneSet.disabled = !exact;
      timezoneFeedback.className = `form-feedback ${exact ? "valid" : value ? "invalid" : ""}`;
      timezoneFeedback.textContent = exact ? "✓ Valid timezone. Ready to save." : value ? "Choose a timezone from the suggestions." : "Start typing to search available timezones.";
    };
    const renderTimezoneSuggestions = () => {
      const matches = timezoneMatches();
      timezoneSuggestionIndex = Math.min(timezoneSuggestionIndex, matches.length - 1);
      timezoneSuggestions.innerHTML = matches.map((value, index) => `<button class="timezone-option ${index === timezoneSuggestionIndex ? "active" : ""}" type="button" role="option" aria-selected="${index === timezoneSuggestionIndex}" data-timezone="${escapeAttribute(value)}">${escapeHtml(value)}</button>`).join("");
      const visible = matches.length > 0 && document.activeElement === timezoneInput;
      timezoneSuggestions.classList.toggle("visible", visible);
      timezoneInput.setAttribute("aria-expanded", String(visible));
      timezoneSuggestions.querySelectorAll("[data-timezone]").forEach((button) => {
        button.addEventListener("mousedown", (event) => event.preventDefault());
        button.addEventListener("click", () => selectTimezone(button.dataset.timezone));
      });
    };
    const closeTimezoneSuggestions = () => {
      timezoneSuggestions.innerHTML = "";
      timezoneSuggestions.classList.remove("visible");
      timezoneInput.setAttribute("aria-expanded", "false");
    };
    const selectTimezone = (value) => {
      timezoneInput.value = value;
      state.configurationDraft = value;
      timezoneSuggestionIndex = -1;
      updateTimezoneFeedback();
      closeTimezoneSuggestions();
      timezoneInput.focus({ preventScroll: true });
    };
    timezoneInput.addEventListener("input", () => {
      state.configurationDraft = timezoneInput.value;
      timezoneSuggestionIndex = -1;
      updateTimezoneFeedback();
      renderTimezoneSuggestions();
    });
    timezoneInput.addEventListener("focus", renderTimezoneSuggestions);
    timezoneInput.addEventListener("blur", () => setTimeout(() => {
      timezoneSuggestions.classList.remove("visible");
      timezoneInput.setAttribute("aria-expanded", "false");
    }, 120));
    timezoneInput.addEventListener("keydown", (event) => {
      const matches = timezoneMatches();
      if (event.key === "ArrowDown" && matches.length) {
        event.preventDefault();
        timezoneSuggestionIndex = Math.min(timezoneSuggestionIndex + 1, matches.length - 1);
        renderTimezoneSuggestions();
      } else if (event.key === "ArrowUp" && matches.length) {
        event.preventDefault();
        timezoneSuggestionIndex = Math.max(timezoneSuggestionIndex - 1, 0);
        renderTimezoneSuggestions();
      } else if (event.key === "Enter") {
        const exact = timezones.includes(timezoneInput.value.trim());
        const selected = timezoneSuggestionIndex >= 0 ? matches[timezoneSuggestionIndex] : (exact ? timezoneInput.value.trim() : null);
        if (selected) {
          event.preventDefault();
          selectTimezone(selected);
        }
      } else if (event.key === "Escape") {
        closeTimezoneSuggestions();
      }
    });
    updateTimezoneFeedback();
    document.getElementById("timezone-form").addEventListener("submit", (event) => {
      event.preventDefault();
      if (!timezoneSet.disabled) mutate("/api/v1/config/timezone", "PUT", { value: timezoneInput.value }, () => { state.configurationDraft = null; });
    });
    document.getElementById("timezone-unset").addEventListener("click", () => mutate("/api/v1/config/timezone", "DELETE", null, () => { state.configurationDraft = null; }));
    document.getElementById("snapshot-create").addEventListener("click", () => mutate("/api/v1/config/snapshot", "POST"));
    bindPagination("snapshots", renderConfiguration);
    document.querySelectorAll("[data-restore-path]").forEach((button) => {
      button.addEventListener("click", async () => {
        const confirmed = await openConfirmation({
          kicker: "Configuration snapshot",
          title: "Restore this snapshot?",
          message: "The current configuration will be preserved before this snapshot is restored.",
          acceptLabel: "Restore snapshot",
        });
        if (confirmed) {
          mutate("/api/v1/config/restore", "POST", { path: button.dataset.restorePath });
        }
      });
    });
  }

  function renderNotice(route) {
    const content = {
      logs: ["Workspace / Logs", "Live logs are coming next", "The navigation is ready. This view will stream bounded stdout and stderr output without interpreting it as HTML."],
    }[route];
    app.innerHTML = `<div class="page-heading"><div><div class="eyebrow">${content[0]}</div><h1>${content[1]}</h1><p>${content[2]}</p></div><button class="button secondary" id="notice-refresh">Refresh workspace <span aria-hidden="true">↻</span></button></div><section class="panel"><div class="notice-panel"><div class="notice-icon">✦</div><div><div class="section-kicker">Next slice</div><h2>Read model is ready</h2><p>Server time zone: <code>${escapeHtml(state.status.timezone.name)}</code> · source: <code>${escapeHtml(state.status.timezone.source)}</code></p></div></div></section>`;
    document.getElementById("notice-refresh").addEventListener("click", loadData);
  }

  function isInteractiveEditing() {
    if (state.route === "configuration" && state.configurationDraft !== null) return true;
    const active = document.activeElement;
    return ["jobs", "logs", "configuration"].includes(state.route)
      && active
      && ["INPUT", "SELECT", "TEXTAREA"].includes(active.tagName);
  }

  function metric(label, value, note, accent) { return `<article class="metric-card ${accent}"><div class="metric-label">${label}</div><div class="metric-value">${value}</div><div class="metric-note">${escapeHtml(note)}</div></article>`; }

  function activeJobMarkup(job) {
    if (!job) return '<div class="empty-state"><div class="empty-icon">○</div><strong>No active job</strong><p>When the scheduler claims work, the current command and owner will appear here.</p></div>';
    return `<div class="job-hero"><div><h3>${escapeHtml(job.name)}</h3><p>${escapeHtml(job.cwd)}</p><div class="job-hero-meta"><span>${escapeHtml(job.user)}</span><span>${escapeHtml(shortId(job.id))}</span><span>${liveTime(job.started_at || job.created_at)}</span></div></div>${stateBadge(job.state)}</div>`;
  }

  function queueMarkup(jobs) {
    if (!jobs.length) return '<div class="empty-state"><div class="empty-icon">≡</div><strong>Queue is clear</strong><p>Committed jobs will show up here in execution order.</p></div>';
    return jobs.map((job, index) => `<div class="queue-row"><span class="queue-order">${String(index + 1).padStart(2, "0")}</span><div class="queue-job"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(job.user)} · ${escapeHtml(shortId(job.id))}</small></div><span class="queue-state">QUEUED</span></div>`).join("");
  }

  function activityMarkup(jobs) {
    const recent = [...jobs].sort((a, b) => new Date(b.finished_at || b.created_at) - new Date(a.finished_at || a.created_at)).slice(0, 4);
    if (!recent.length) return '<div class="empty-state"><div class="empty-icon">⌁</div><strong>No activity yet</strong><p>Create a DRAFT job with the CLI to start building your workspace.</p></div>';
    return recent.map((job) => `<div class="activity-item"><div class="activity-line"></div><div class="activity-copy"><strong>${escapeHtml(job.name)} · ${escapeHtml(job.state)}</strong><small>${escapeHtml(job.user)} · ${liveTime(job.finished_at || job.created_at)}</small></div></div>`).join("");
  }

  function jobRow(job) {
    const selected = state.jobDetails.selectedId === job.id;
    return `<tr class="job-row${selected ? " selected" : ""}" data-job-open="${escapeAttribute(job.id)}" tabindex="0" aria-label="Open details for ${escapeAttribute(job.name)}"><td><div class="job-name"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(shortId(job.id))}</small></div></td><td>${escapeHtml(job.user)}</td><td class="path-cell" title="${escapeAttribute(job.cwd)}">${escapeHtml(job.cwd)}</td><td>${stateBadge(job.state)}</td><td class="mono">${job.queue_order == null ? "—" : job.queue_order}</td><td>${liveTime(job.created_at)}</td></tr>`;
  }

  function emptyTableRow(message, colspan = 5) { return `<tr><td colspan="${colspan}"><div class="empty-state"><div class="empty-icon">○</div><strong>${escapeHtml(message)}</strong></div></td></tr>`; }

  function stateBadge(value) { const key = String(value || "").toLowerCase(); return `<span class="state-badge state-${key}">${escapeHtml(value || "UNKNOWN")}</span>`; }

  function isTerminalState(value) { return ["SUCCEEDED", "FAILED", "CANCELLED", "LOST"].includes(value); }

  function routeTitle(route) { return { overview: "Overview", jobs: "Jobs", queue: "Queue", logs: "Logs", configuration: "Configuration" }[route] || "Overview"; }

  function shortId(id) { return id ? `${String(id).slice(0, 8)}…` : "—"; }

  function formatDate(value) {
    if (!value) return "—";
    const date = new Date(value);
    if (Number.isNaN(date.valueOf())) return "—";
    const options = { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" };
    if (state.timezone && state.timezone.name) options.timeZone = state.timezone.name;
    try {
      return new Intl.DateTimeFormat(undefined, options).format(date);
    } catch (_) {
      delete options.timeZone;
      return new Intl.DateTimeFormat(undefined, options).format(date);
    }
  }

  function liveTime(value) {
    if (!value) return "—";
    const serialized = value instanceof Date ? value.toISOString() : String(value);
    return `<time class="live-time" datetime="${escapeAttribute(serialized)}" data-live-time="${escapeAttribute(serialized)}">${formatDate(serialized)}</time>`;
  }

  function refreshLiveTimes() {
    document.querySelectorAll("[data-live-time]").forEach((element) => {
      element.textContent = formatDate(element.dataset.liveTime);
    });
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
      const start = Math.min(snapshot.start, end);
      input.setSelectionRange(start, end);
    }
  }

  function escapeHtml(value) { return String(value ?? "").replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" }[char])); }

  function escapeAttribute(value) { return escapeHtml(value); }

  function updateSchedulerPill() {
    const pill = document.getElementById("scheduler-pill");
    const label = document.getElementById("scheduler-label");
    const running = Boolean(state.status && state.status.scheduler.running);
    pill.classList.toggle("running", running);
    pill.classList.toggle("stopped", !running);
    label.textContent = running ? "Scheduler running" : "Scheduler stopped";
  }

  function updateConnection(online) {
    const dot = document.getElementById("connection-dot");
    const label = document.getElementById("connection-label");
    const detail = document.getElementById("connection-detail");
    dot.classList.toggle("online", online);
    dot.classList.toggle("error", !online);
    label.textContent = online ? "Server connected" : "Connection issue";
    detail.textContent = online ? "State synced just now" : "Retry to reconnect";
  }

  function showToast(message, error = false) {
    const region = document.getElementById("toast-region");
    const toast = document.createElement("div");
    toast.className = `toast${error ? " error" : ""}`;
    toast.textContent = message;
    region.appendChild(toast);
    setTimeout(() => toast.remove(), 4200);
  }

  function listViewportBottom() {
    const mobile = window.matchMedia("(max-width: 760px)").matches;
    const main = document.querySelector(".main-content");
    const bottom = mobile ? window.innerHeight - 70 : main.getBoundingClientRect().top + main.clientHeight;
    // Account for an existing outer scroll position while measuring a rerender.
    const scrollOffset = mobile ? window.scrollY : main.scrollTop;
    return bottom - scrollOffset - parseFloat(getComputedStyle(app).paddingBottom);
  }

  window.addEventListener("resize", () => {
    if (state.loaded && ["jobs", "queue"].includes(state.route)) render();
  });
  window.addEventListener("hashchange", () => { state.route = location.hash.slice(1) || "overview"; if (state.loaded) render(); });
  document.getElementById("refresh-button").addEventListener("click", loadData);
  tokenForm.addEventListener("submit", (event) => {
    event.preventDefault();
    const token = document.getElementById("token-input").value.trim();
    if (!token) return;
    sessionStorage.setItem("stoker-ui-token", token);
    tokenDialog.close();
    loadData();
  });
  confirmCancel.addEventListener("click", () => closeConfirmation(false));
  confirmAccept.addEventListener("click", () => closeConfirmation(true));
  confirmDialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    closeConfirmation(false);
  });
  jobDialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    closeJobDialog();
  });
  jobDialog.addEventListener("click", (event) => {
    if (event.target === jobDialog) closeJobDialog();
  });
  jobDetailDialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    closeJobDetail();
  });
  jobDetailDialog.addEventListener("click", (event) => {
    if (event.target === jobDetailDialog) closeJobDetail();
  });

  loadData();
  setInterval(() => { if (document.visibilityState === "visible") loadData(); }, 2000);
})();
