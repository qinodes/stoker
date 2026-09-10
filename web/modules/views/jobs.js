import { emptyState, jobRow, pageHeading, pagination } from "../components.js";
import { escapeAttribute, escapeHtml, isTerminalState, limitUnicode, liveTime, stateBadge } from "../formatters.js";
import { JOBS_PAGE_SIZE, pageInfo } from "../state.js";

export function renderJobs(state) {
  const query = state.filters.search.toLowerCase();
  const jobs = state.jobs.filter((job) => {
    const searchMatch = !query || [job.name, job.user, job.cwd, job.id].some((value) => String(value || "").toLowerCase().includes(query));
    return searchMatch && (!state.filters.user || job.user === state.filters.user) && (!state.filters.state || job.state === state.filters.state);
  });
  const page = pageInfo(jobs, state.pagination.jobs, JOBS_PAGE_SIZE);
  state.pagination.jobs = page.page;
  const owners = [...new Set(state.jobs.map((job) => job.user))].sort();
  const states = [...new Set(state.jobs.map((job) => job.state))].sort();
  const rows = page.items.length
    ? page.items.map((job) => jobRow(job, state.timezone?.name)).join("")
    : `<tr><td colspan="6">${emptyState("○", "No jobs match these filters")}</td></tr>`;
  return `${pageHeading("Workspace / Jobs", "Jobs", "Create, inspect, filter, and clean scheduled work.", '<button class="button danger" data-action="clean-jobs">Clean job history</button><button class="button primary" data-action="new-job">New job ＋</button>')}
    <div class="toolbar"><input class="search-input" id="job-search" value="${escapeAttribute(state.filters.search)}" placeholder="Search jobs"><div class="filter-group"><select class="select-input" id="job-owner-filter"><option value="">All owners</option>${owners.map((owner) => `<option ${owner === state.filters.user ? "selected" : ""}>${escapeHtml(owner)}</option>`).join("")}</select><select class="select-input" id="job-state-filter"><option value="">All states</option>${states.map((value) => `<option ${value === state.filters.state ? "selected" : ""}>${escapeHtml(value)}</option>`).join("")}</select></div></div>
    <section class="panel jobs-table"><div class="table-wrap"><table><thead><tr><th>Job</th><th>Owner</th><th>Working directory</th><th>State</th><th>Order</th><th>Created</th></tr></thead><tbody>${rows}</tbody></table></div>${pagination("jobs", page)}</section>`;
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
  return `<div class="job-dialog-header"><div><div class="dialog-kicker">New scheduled work</div><h2 id="job-dialog-title">Create job</h2></div><button class="icon-button" data-action="close-job-form" aria-label="Close">×</button></div>
    <form id="job-form"><div class="job-form-grid"><div class="job-field"><label for="job-user">Owner</label><input class="text-input" id="job-user" maxlength="${limits.user}" value="${escapeAttribute(draft.user)}" required></div><div class="job-field"><label for="job-name">Job name</label><input class="text-input" id="job-name" maxlength="${limits.name}" value="${escapeAttribute(draft.name)}" required></div></div>
    <div class="job-field"><label for="job-cwd">Working directory</label><div class="path-input-row"><input class="text-input" id="job-cwd" value="${escapeAttribute(draft.cwd)}" required><button class="button secondary" data-action="browse-directory" type="button">Browse</button></div></div>
    ${state.filesystem.roots ? `<div class="fs-browser"><div class="fs-path-row"><input class="text-input" id="fs-path" value="${escapeAttribute(state.filesystem.inputPath || browser?.path || "")}"><button class="button secondary" data-action="open-directory" type="button">Open</button></div>${state.filesystem.error ? `<div class="form-feedback invalid">${escapeHtml(state.filesystem.error)}</div>` : ""}<div class="fs-directory-list">${state.filesystem.loading ? emptyState("⌁", "Loading directories") : directories.map((entry) => `<button class="fs-directory-row" type="button" data-directory="${escapeAttribute(entry.path)}"><span class="fs-folder-glyph">◆</span><span>${escapeHtml(entry.name)}</span><span class="fs-row-arrow">›</span></button>`).join("") || emptyState("○", "No subdirectories")}</div><button class="button small primary" data-action="choose-directory" type="button">Use this directory</button></div>` : ""}
    <div class="job-field"><label for="job-command">Command</label><input class="text-input mono" id="job-command" value="${escapeAttribute(draft.command)}" required></div><div class="job-field"><label for="job-description">Description</label><textarea class="text-input" id="job-description" maxlength="${limits.description}">${escapeHtml(draft.description)}</textarea></div><div class="dialog-actions"><button class="button secondary" data-action="close-job-form" type="button">Cancel</button><button class="button primary" type="submit">Create job</button></div></form>`;
}

export function renderJobDetail(job, state, editing = false) {
  if (!job) return emptyState("○", "Job no longer exists");
  const canCommit = job.state === "DRAFT";
  const canCancel = ["DRAFT", "QUEUED", "STARTING", "RUNNING"].includes(job.state);
  const description = job.description || "No description provided.";
  return `<div class="job-detail-header"><div><div class="dialog-kicker">Job detail</div><h2 id="job-detail-title">${escapeHtml(job.name)}</h2><p class="mono">${escapeHtml(job.id)}</p></div><button class="icon-button" data-action="close-job-detail">×</button></div><div class="job-detail-status"><div>${stateBadge(job.state)}</div><span>${escapeHtml(job.user)}</span><span>${escapeHtml(job.cwd)}</span></div>
    <section class="job-detail-section"><div class="section-heading-row"><h3>Description</h3><button class="button small secondary" data-action="edit-description">${editing ? "Cancel" : "Edit"}</button></div>${editing ? `<form id="description-form"><textarea class="text-input job-description-editor" id="description-input" maxlength="${state.config?.max_job_description_length || 200}">${escapeHtml(job.description || "")}</textarea><div class="dialog-actions"><button class="button primary" type="submit">Save description</button></div></form>` : `<p class="job-description-preview">${escapeHtml(limitUnicode(description, 1000))}</p>`}</section>
    <section class="job-detail-section"><h3>Command</h3><pre class="config-json">${escapeHtml(job.command_line || job.command.join(" "))}</pre></section><section class="job-detail-section"><div class="job-detail-grid"><div><small>Created</small><strong>${liveTime(job.created_at, state.timezone?.name)}</strong></div><div><small>Finished</small><strong>${liveTime(job.finished_at, state.timezone?.name)}</strong></div><div><small>Exit code</small><strong>${job.exit_code ?? "—"}</strong></div><div><small>PID</small><strong>${job.pid ?? "—"}</strong></div></div></section>
    <div class="job-detail-actions"><button class="button secondary" data-action="view-job-logs" data-job-id="${job.id}">View logs</button><span class="job-action-spacer"></span>${canCommit ? `<button class="button primary" data-job-action="commit" data-job-id="${job.id}">Commit job</button>` : ""}${canCancel && !isTerminalState(job.state) ? `<button class="button danger" data-job-action="cancel" data-job-id="${job.id}">Cancel job</button>` : ""}</div>`;
}
