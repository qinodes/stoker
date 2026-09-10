import { emptyState, jobRow, pageHeading, pagination } from "../components.js";
import { escapeAttribute, escapeHtml, isTerminalState, limitUnicode, liveTime, stateBadge } from "../formatters.js";
import { JOBS_PAGE_SIZE, pageInfo } from "../state.js";

export function renderJobs(state) {
  const query = state.filters.search.toLowerCase();
  const filtered = state.jobs.filter((job) => {
    const searchMatch = !query || [job.name, job.user, job.cwd, job.id].some((value) => String(value || "").toLowerCase().includes(query));
    return searchMatch && (!state.filters.user || job.user === state.filters.user) && (!state.filters.state || job.state === state.filters.state);
  });
  const page = pageInfo(filtered, state.pagination.jobs, JOBS_PAGE_SIZE);
  state.pagination.jobs = page.page;
  const owners = [...new Set(state.jobs.map((job) => job.user))].sort();
  const terminalCount = state.jobs.filter((job) => isTerminalState(job.state)).length;
  const rows = page.items.length
    ? page.items.map((job) => jobRow(job, state.timezone?.name)).join("")
    : `<tr><td colspan="6">${emptyState("○", "No jobs match these filters")}</td></tr>`;
  return `
    <div class="page-heading"><div><div class="eyebrow">Workspace / Jobs</div><h1>All jobs</h1><p>Search every submission, inspect its working directory, and keep an eye on its lifecycle.</p></div><div class="page-actions"><button class="button primary" data-action="new-job" type="button">＋ New job</button><button class="button secondary" data-action="refresh">Refresh jobs <span aria-hidden="true">↻</span></button><button class="button danger" data-action="clean-jobs" type="button" ${terminalCount ? "" : "disabled"}>Clean job history${terminalCount ? ` (${terminalCount})` : ""}</button></div></div>
    <div class="toolbar"><input class="search-input" id="job-search" type="search" autocomplete="off" spellcheck="false" placeholder="Search job name, owner, ID, or path" value="${escapeAttribute(state.filters.search)}" aria-label="Search jobs"><div class="filter-group"><select class="select-input" id="user-filter" aria-label="Filter by owner"><option value="">All owners</option>${owners.map((owner) => `<option value="${escapeAttribute(owner)}" ${owner === state.filters.user ? "selected" : ""}>${escapeHtml(owner)}</option>`).join("")}</select><select class="select-input" id="state-filter" aria-label="Filter by state"><option value="">All states</option>${["DRAFT", "QUEUED", "STARTING", "RUNNING", "CANCELLING", "SUCCEEDED", "FAILED", "CANCELLED", "LOST"].map((value) => `<option value="${value}" ${value === state.filters.state ? "selected" : ""}>${value}</option>`).join("")}</select></div></div>
    <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>${filtered.length} visible job${filtered.length === 1 ? "" : "s"}</h2><p>Server state · ${liveTime(new Date(), state.timezone?.name)}</p></div></div></div><div class="table-wrap"><table class="data-table jobs-table"><thead><tr><th>Job name</th><th>Owner</th><th>Path</th><th>State</th><th>Queue</th><th>Created</th></tr></thead><tbody>${rows}</tbody></table></div>${pagination("jobs", page)}</section>`;
}

export function renderJobForm(state) {
  const limits = {
    name: state.config?.max_job_name_length || 128,
    user: state.config?.max_job_user_length || 50,
    description: state.config?.max_job_description_length || 200,
  };
  const draft = state.jobDraft;
  const browser = state.filesystem.current;
  const directories = browser?.directories || [];
  return `<div class="dialog-card job-card">
    <div class="dialog-kicker">Workspace / Jobs</div>
    <div class="job-dialog-heading"><div><h2 id="job-dialog-title">New job</h2><p>Create a draft job. Commit it when you're ready to run.</p></div><button class="dialog-close" data-action="close-job-form" type="button" aria-label="Close new job">×</button></div>
    <form id="new-job-form" novalidate>
      <div class="job-form-grid"><div class="job-field"><label for="job-user">Owner</label><div class="job-user-picker"><input class="text-input" id="job-user" maxlength="${limits.user}" value="${escapeAttribute(draft.user)}" autocomplete="off" autocapitalize="off" aria-autocomplete="list" aria-controls="job-user-suggestions" aria-expanded="false" required><div class="job-suggestions" id="job-user-suggestions" role="listbox"></div></div><small class="form-help">Maximum ${limits.user} characters.</small></div><div class="job-field"><label for="job-name">Job name</label><input class="text-input" id="job-name" maxlength="${limits.name}" value="${escapeAttribute(draft.name)}" autocomplete="off" required><small class="form-help">Maximum ${limits.name} characters.</small></div></div>
      <div class="job-field"><label for="job-cwd">Working directory</label><div class="path-input-row"><input class="text-input mono-input" id="job-cwd" value="${escapeAttribute(draft.cwd)}" placeholder="Choose a folder on the Stoker host" autocomplete="off" autocapitalize="off" spellcheck="false" required><button class="button secondary" data-action="browse-directory" type="button">Browse</button></div><small class="form-help">Folders are read from the Stoker host.</small></div>
      ${browser || state.filesystem.loading ? `<div class="fs-browser"><div class="fs-path-row"><input class="text-input" id="fs-path" value="${escapeAttribute(state.filesystem.inputPath || browser?.path || "")}"><button class="button secondary" data-action="open-directory" type="button">Open</button></div>${state.filesystem.error ? `<div class="form-feedback invalid">${escapeHtml(state.filesystem.error)}</div>` : ""}<div class="fs-directory-list">${state.filesystem.loading ? emptyState("⌁", "Loading directories") : directories.map((entry) => `<button class="fs-directory-row" type="button" data-directory="${escapeAttribute(entry.path)}"><span class="fs-folder-glyph">◆</span><span>${escapeHtml(entry.name)}</span><span class="fs-row-arrow">›</span></button>`).join("") || emptyState("○", "No subdirectories")}</div>${browser ? '<button class="button small primary" data-action="choose-directory" type="button">Use this directory</button>' : ""}</div>` : ""}
      <div class="job-field"><label for="job-command">Command</label><textarea class="command-input" id="job-command" rows="4" placeholder="cargo build --release" required>${escapeHtml(draft.command)}</textarea><small class="form-help">Enter the command only; Stoker will keep the same quoting rules as <code>stoker add --cmd</code>.</small></div>
      <div class="job-field"><label for="job-description">Description <span class="field-optional">Optional</span></label><textarea class="description-input" id="job-description" rows="4" maxlength="${limits.description}" placeholder="What is this job for?">${escapeHtml(draft.description)}</textarea><small class="form-help">Maximum ${limits.description} characters. <span id="job-description-count">${[...draft.description].length}/${limits.description}</span></small></div>
      <div class="form-feedback ${state.filesystem.error ? "invalid" : ""}" aria-live="polite">${escapeHtml(state.filesystem.error)}</div>
      <div class="dialog-actions"><button class="button secondary" data-action="close-job-form" type="button">Cancel</button><button class="button primary" type="submit">Create draft</button></div>
    </form>
  </div>`;
}

export function renderJobDetail(job, state, editing = false) {
  if (!job) return `<div class="job-detail-card">${emptyState("○", "Job no longer exists")}</div>`;
  const canCommit = job.state === "DRAFT";
  const canCancel = ["DRAFT", "QUEUED", "STARTING", "RUNNING"].includes(job.state);
  const description = job.description || "";
  const command = job.command_line || (Array.isArray(job.command) ? job.command.join(" ") : job.command) || "—";
  const detail = state.selectedJobDetail || {};
  const directoryStatus = detail.working_directory_status || (["RUNNING", "STARTING", "CANCELLING"].includes(job.state) ? "Used by the active process" : "Stored submission path");
  const timeline = [["Created", job.created_at], ["Committed", job.committed_at], ["Started", job.started_at], ["Finished", job.finished_at]];
  const timelineMarkup = timeline.map(([label, value]) => `<div class="timeline-item${value ? " has-value" : ""}"><span class="timeline-dot" aria-hidden="true"></span><span class="timeline-label">${label}</span><span class="timeline-time">${value ? liveTime(value, state.timezone?.name) : "Not recorded"}</span></div>`).join("");
  return `<div class="job-detail-card">
    <div class="job-detail-heading"><div><div class="dialog-kicker">Workspace / Jobs</div><h2 id="job-detail-title">${escapeHtml(job.name)}</h2><p class="job-detail-id mono"><span>${escapeHtml(job.id)}</span><button class="copy-id-button" data-action="copy-job-id" data-job-id="${escapeAttribute(job.id)}" type="button" title="Copy job ID" aria-label="Copy job ID"></button></p></div><button class="dialog-close" data-action="close-job-detail" type="button" aria-label="Close job details">×</button></div>
    <div class="job-detail-status"><div><span class="section-kicker">Current state</span><div class="job-detail-state">${stateBadge(job.state)}</div></div><div class="job-detail-status-meta"><span class="job-detail-directory-status">${escapeHtml(directoryStatus)}</span></div></div>
    <section class="job-detail-section"><div class="section-kicker">Job overview</div><div class="job-detail-grid"><div><span class="detail-label">Owner</span><strong>${escapeHtml(job.user)}</strong></div><div><span class="detail-label">Queue order</span><strong>${job.queue_order ?? "—"}</strong></div><div class="job-detail-wide"><span class="detail-label">Working directory</span><code>${escapeHtml(job.cwd)}</code></div><div class="job-detail-wide"><span class="detail-label">Command</span><pre class="job-command">${escapeHtml(command)}</pre></div></div></section>
    <section class="job-detail-section job-description-section"><div class="section-heading-row"><div class="section-kicker">Description</div><button class="button small secondary" data-action="edit-description" type="button">${editing ? "Cancel" : (description ? "Edit" : "Add description")}</button></div>${editing ? `<form id="description-form"><textarea class="description-input job-description-editor" id="description-input" maxlength="${state.config?.max_job_description_length || 200}" rows="5">${escapeHtml(description)}</textarea><div class="description-editor-meta"><small class="form-help">${[...description].length}/${state.config?.max_job_description_length || 200}</small><div class="description-editor-actions"><button class="button small primary" type="submit">Save</button></div></div></form>` : `<p class="job-description-preview${description ? "" : " empty"}">${escapeHtml(limitUnicode(description || "No description provided.", 1000))}</p>`}</section>
    <section class="job-detail-section"><div class="timeline-heading"><div class="section-kicker">Timeline</div>${detail.display_timezone ? `<span class="job-detail-timezone">Times shown in ${escapeHtml(detail.display_timezone)}</span>` : ""}</div><div class="job-timeline">${timelineMarkup}</div></section>
    <div class="job-detail-actions"><button class="button secondary" data-action="view-job-logs" data-job-id="${job.id}">View logs</button><span class="job-action-spacer"></span>${canCommit ? `<button class="button primary" data-job-action="commit" data-job-id="${job.id}">Commit job</button>` : ""}${canCancel && !isTerminalState(job.state) ? `<button class="button danger" data-job-action="cancel" data-job-id="${job.id}">Cancel job</button>` : ""}</div>
  </div>`;
}
