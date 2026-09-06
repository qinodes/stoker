(() => {
  "use strict";

  const MAX_LOG_KB = 256;

  const state = {
    config: null,
    settings: null,
    status: null,
    timezone: null,
    jobs: [],
    queue: { jobs: [], locked: false },
    logs: { jobId: "", stream: "stdout", data: null, error: null },
    route: location.hash.slice(1) || "overview",
    filters: { search: "", user: "", state: "" },
    loaded: false,
    loading: false,
    renderAfterLoad: false,
    configurationDraft: null,
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

  function openConfirmation({ kicker, title, message, acceptLabel }) {
    return new Promise((resolve) => {
      confirmationResolver = resolve;
      confirmKicker.textContent = kicker;
      confirmTitle.textContent = title;
      confirmMessage.textContent = message;
      confirmAccept.textContent = acceptLabel;
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

  function renderJobs() {
    const focusSnapshot = captureFocus();
    const filtered = state.jobs.filter((job) => {
      const search = state.filters.search.toLowerCase();
      const matchesSearch = !search || [job.name, job.user, job.id, job.cwd].some((value) => String(value || "").toLowerCase().includes(search));
      return matchesSearch && (!state.filters.user || job.user === state.filters.user) && (!state.filters.state || job.state === state.filters.state);
    });
    const users = [...new Set(state.jobs.map((job) => job.user))].sort();
    const terminalCount = state.jobs.filter((job) => isTerminalState(job.state)).length;
    app.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">Workspace / Jobs</div><h1>All jobs</h1><p>Search every submission, inspect its working directory, and keep an eye on its lifecycle.</p></div><div class="page-actions"><button class="button secondary" id="jobs-refresh">Refresh jobs <span aria-hidden="true">↻</span></button><button class="button danger" id="jobs-clean" type="button" ${terminalCount ? "" : "disabled"}>Clean job history${terminalCount ? ` (${terminalCount})` : ""}</button></div></div>
      <div class="toolbar"><input class="search-input" id="job-search" type="search" placeholder="Search job name, owner, ID, or path" value="${escapeAttribute(state.filters.search)}" aria-label="Search jobs"><div class="filter-group"><select class="select-input" id="user-filter" aria-label="Filter by owner"><option value="">All owners</option>${users.map((user) => `<option value="${escapeAttribute(user)}" ${user === state.filters.user ? "selected" : ""}>${escapeHtml(user)}</option>`).join("")}</select><select class="select-input" id="state-filter" aria-label="Filter by state"><option value="">All states</option>${["DRAFT", "QUEUED", "STARTING", "RUNNING", "CANCELLING", "SUCCEEDED", "FAILED", "CANCELLED", "LOST"].map((value) => `<option value="${value}" ${value === state.filters.state ? "selected" : ""}>${value}</option>`).join("")}</select></div></div>
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>${filtered.length} visible job${filtered.length === 1 ? "" : "s"}</h2><p>Server state · ${liveTime(new Date())}</p></div></div></div><div class="table-wrap"><table class="data-table jobs-table"><thead><tr><th>Job name</th><th>Owner</th><th>Path</th><th>State</th><th>Queue</th><th>Created</th></tr></thead><tbody>${filtered.length ? filtered.map(jobRow).join("") : emptyTableRow("No jobs match these filters.", 6)}</tbody></table></div></section>`;
    document.getElementById("jobs-refresh").addEventListener("click", loadData);
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
    document.getElementById("job-search").addEventListener("input", (event) => { state.filters.search = event.target.value; renderJobs(); });
    document.getElementById("user-filter").addEventListener("change", (event) => { state.filters.user = event.target.value; renderJobs(); });
    document.getElementById("state-filter").addEventListener("change", (event) => { state.filters.state = event.target.value; renderJobs(); });
    restoreFocus(focusSnapshot);
  }

  function renderQueue() {
    const jobs = state.queue.jobs;
    const locked = Boolean(state.queue.locked);
    app.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">Workspace / Queue</div><h1>Execution queue</h1><p>Lock the global queue before changing its order. Every move is checked against the server’s latest state.</p></div><div class="page-actions"><button class="button secondary" id="queue-refresh">Refresh queue <span aria-hidden="true">↻</span></button><button class="button primary" id="queue-lock" ${locked ? "disabled" : ""}>Lock queue</button><button class="button secondary" id="queue-unlock" ${locked ? "" : "disabled"}>Unlock queue</button></div></div>
      ${locked ? '<div class="queue-lock-banner"><div><strong>Queue is locked</strong><small>This global lock is visible to every connected client. Use the arrows below to adjust order.</small></div><span aria-hidden="true">🔒</span></div>' : '<div class="queue-unlock-banner"><div><strong>Queue is unlocked</strong><small>Lock the queue to enable reorder controls and prevent the scheduler from claiming work during edits.</small></div><span aria-hidden="true">↕</span></div>'}
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>${jobs.length} queued job${jobs.length === 1 ? "" : "s"}</h2><p>Ordered by queue position · server state</p></div></div></div><div class="queue-table"><table class="data-table queue-order-table"><thead><tr><th>#</th><th>Job name</th><th>Owner</th><th>Path</th><th>State</th><th>Move</th></tr></thead><tbody>${jobs.length ? jobs.map((job, index) => `<tr><td class="mono">${index + 1}</td><td><div class="job-name"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(shortId(job.id))}</small></div></td><td>${escapeHtml(job.user)}</td><td class="path-cell" title="${escapeAttribute(job.cwd)}">${escapeHtml(job.cwd)}</td><td>${stateBadge(job.state)}</td><td><div class="move-controls"><button class="move-button" type="button" data-queue-move="${escapeAttribute(job.id)}" data-target-order="${index}" aria-label="Move ${escapeAttribute(job.name)} up" ${!locked || index === 0 ? "disabled" : ""}>↑</button><button class="move-button" type="button" data-queue-move="${escapeAttribute(job.id)}" data-target-order="${index + 2}" aria-label="Move ${escapeAttribute(job.name)} down" ${!locked || index === jobs.length - 1 ? "disabled" : ""}>↓</button></div></td></tr>`).join("") : emptyTableRow("The queue is clear. Commit a DRAFT job from the Jobs view or CLI.", 6)}</tbody></table></div></section>`;
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
    const data = state.logs.data;
    const stream = state.logs.stream === "stderr" ? "stderr" : "stdout";
    const streamAvailable = data ? Boolean(data[`${stream}_available`]) : false;
    const streamText = data ? data[stream] || "" : "";
    const streamTruncated = data ? Boolean(data[`${stream}_truncated`]) : false;
    const message = state.logs.error || (data && data.message) || (!availableJobs.length ? "No jobs are available yet. Add a job with the CLI first." : (!streamAvailable && selectedJob ? `No ${stream} log is available for this job yet.` : ""));
    const jobOptions = availableJobs.length ? availableJobs.map((job) => `<option value="${escapeAttribute(job.id)}" ${job.id === state.logs.jobId ? "selected" : ""}>${escapeHtml(job.name)} · ${escapeHtml(shortId(job.id))} · ${escapeHtml(job.state)}</option>`).join("") : '<option value="">No jobs available</option>';
    const output = selectedJob && data && streamAvailable ? streamText || "(No output written yet.)" : "Select a job to inspect its output.";
    app.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">Workspace / Logs</div><h1>Job logs</h1><p>Choose a job, then switch between stdout and stderr. Logs are shown as plain text and are refreshed automatically every 2 seconds.</p></div><button class="button secondary" id="logs-refresh">Refresh logs <span aria-hidden="true">↻</span></button></div>
      <section class="panel log-guide"><div class="panel-header"><div class="panel-title"><div><h2>How to use Logs</h2><p>Logs become available after a committed job starts running.</p></div></div></div><div class="log-guide-grid"><div><span class="guide-number">1</span><div><strong>Choose a Job</strong><small>Select any job from the list below, including queued jobs to see why output is not available yet.</small></div></div><div><span class="guide-number">2</span><div><strong>Pick an output stream</strong><small>Use stdout for normal command output or stderr for errors and diagnostics.</small></div></div><div><span class="guide-number">3</span><div><strong>Watch the latest output</strong><small>Running jobs refresh while this page is open; large logs show the latest 256 KB.</small></div></div></div></section>
      <section class="panel logs-panel"><div class="logs-toolbar"><label for="log-job-select">Job</label><select class="select-input log-job-select" id="log-job-select" aria-label="Choose a job"><option value="">Choose a job…</option>${jobOptions}</select><div class="log-tabs" role="tablist" aria-label="Log stream"><button class="log-tab ${stream === "stdout" ? "active" : ""}" type="button" data-log-stream="stdout" role="tab" aria-selected="${stream === "stdout"}">stdout</button><button class="log-tab ${stream === "stderr" ? "active" : ""}" type="button" data-log-stream="stderr" role="tab" aria-selected="${stream === "stderr"}">stderr</button></div></div>${selectedJob ? `<div class="log-context"><div><strong>${escapeHtml(selectedJob.name)}</strong><small>${escapeHtml(selectedJob.user)} · ${escapeHtml(selectedJob.cwd)}</small></div>${stateBadge(selectedJob.state)}</div>` : ""}${message ? `<div class="log-message"><span aria-hidden="true">ⓘ</span><span>${escapeHtml(message)}</span></div>` : ""}<pre class="log-output" aria-live="polite">${escapeHtml(output)}</pre>${streamTruncated ? `<div class="log-footnote">Showing the latest ${MAX_LOG_KB} KB of ${stream} output.</div>` : ""}</section>`;
    document.getElementById("logs-refresh").addEventListener("click", loadData);
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
    const snapshotMarkup = snapshots.length ? snapshots.map((snapshot) => `
      <div class="snapshot-row">
        <div class="snapshot-copy"><strong>${snapshot.created_at ? liveTime(snapshot.created_at) : "Invalid snapshot"}</strong><small>${escapeHtml(snapshot.reason || snapshot.error || snapshot.path)}</small></div>
        <div class="snapshot-actions">${snapshot.valid ? `<button class="button small secondary" type="button" data-restore-path="${escapeAttribute(snapshot.path)}">Restore</button>` : '<span class="state-badge state-failed">Invalid</span>'}</div>
      </div>`).join("") : '<div class="empty-state compact"><strong>No snapshots yet</strong><p>Save a manual snapshot before making a configuration change.</p></div>';
    app.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">Workspace / Configuration</div><h1>Workspace settings</h1><p>Manage the same timezone and configuration snapshots available from the CLI.</p></div><button class="button secondary" id="configuration-refresh">Refresh configuration <span aria-hidden="true">↻</span></button></div>
      <div class="config-layout">
        <section class="panel config-card"><div class="panel-header"><div class="panel-title"><div><h2>Timezone</h2><p>Used when displaying timestamps in the UI and CLI.</p></div></div></div><div class="config-summary"><div><span class="config-label">Effective timezone</span><strong>${escapeHtml(timezone.name)}</strong></div><div><span class="config-label">Source</span><span class="config-value">${escapeHtml(timezone.source)}</span></div><div><span class="config-label">Configured value</span><span class="config-value">${escapeHtml(configuredTimezone || "Not set · follows system")}</span></div></div><form class="config-form" id="timezone-form"><label for="timezone-input">Set IANA timezone</label><div class="timezone-picker"><input class="text-input" id="timezone-input" name="timezone" value="${escapeAttribute(configuredTimezone)}" placeholder="Type Tokyo, Taipei, UTC…" autocomplete="off" role="combobox" aria-autocomplete="list" aria-controls="timezone-suggestions" aria-expanded="false"><div class="timezone-suggestions" id="timezone-suggestions" role="listbox"></div></div><div id="timezone-feedback" class="form-feedback" aria-live="polite"></div><div class="form-row"><button class="button primary" id="timezone-set" type="submit" disabled>Set timezone</button><button class="button secondary" type="button" id="timezone-unset">Use system</button></div><small class="form-help">Type to search ${timezones.length} IANA timezones, then click a suggestion or press Enter.</small></form></section>
        <section class="panel config-card"><div class="panel-header"><div class="panel-title"><div><h2>Configuration snapshots</h2><p>Restore a known configuration without touching jobs.</p></div><button class="button small secondary" id="snapshot-create" type="button">Create snapshot</button></div></div><div class="snapshot-list">${snapshotMarkup}</div><div class="config-paths"><span>Config: <code>${escapeHtml(settings.config_path)}</code></span><span>Snapshots: <code>${escapeHtml(settings.snapshot_dir)}</code></span></div></section>
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
    return `<tr><td><div class="job-name"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(shortId(job.id))}</small></div></td><td>${escapeHtml(job.user)}</td><td class="path-cell" title="${escapeAttribute(job.cwd)}">${escapeHtml(job.cwd)}</td><td>${stateBadge(job.state)}</td><td class="mono">${job.queue_order == null ? "—" : job.queue_order}</td><td>${liveTime(job.created_at)}</td></tr>`;
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

  loadData();
  setInterval(() => { if (document.visibilityState === "visible") loadData(); }, 2000);
})();
