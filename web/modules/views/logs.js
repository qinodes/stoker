import { emptyState, pageHeading } from "../components.js";
import { escapeAttribute, escapeHtml } from "../formatters.js";

export function renderLogs(state) {
  const query = state.logs.search.toLowerCase();
  const choices = state.jobs.filter((job) => !query || [job.name, job.user, job.id].some((value) => String(value).toLowerCase().includes(query)));
  const data = state.logs.data;
  const stream = state.logs.stream;
  const output = data ? data[stream] : "";
  return `${pageHeading("Workspace / Logs", "Job logs", "Inspect bounded stdout and stderr without rendering log content as HTML.", '<button class="button secondary" data-action="refresh">Refresh</button>')}<section class="panel logs-panel"><div class="logs-toolbar"><label for="log-job-search">Job</label><input class="search-input log-job-search" id="log-job-search" value="${escapeAttribute(state.logs.search)}" placeholder="Filter jobs"><select class="select-input log-job-select" id="log-job-select"><option value="">Choose a job</option>${choices.map((job) => `<option value="${job.id}" ${job.id === state.logs.jobId ? "selected" : ""}>${escapeHtml(job.name)} · ${escapeHtml(job.user)}</option>`).join("")}</select><div class="log-tabs"><button class="log-tab ${stream === "stdout" ? "active" : ""}" data-log-stream="stdout">stdout</button><button class="log-tab ${stream === "stderr" ? "active" : ""}" data-log-stream="stderr">stderr</button></div></div>${state.logs.error ? `<div class="form-feedback invalid">${escapeHtml(state.logs.error)}</div>` : ""}${data ? `<div class="log-context"><strong>${escapeHtml(data.job.name)}</strong><span>${escapeHtml(data.job.state)}</span></div>${data.message ? `<p class="log-message">${escapeHtml(data.message)}</p>` : ""}<pre class="log-output">${escapeHtml(output || `${stream} has no output.`)}</pre>` : emptyState("⌁", "Choose a job", "Its bounded stdout and stderr will appear here.")}</section>`;
}
