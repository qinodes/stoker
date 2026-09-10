import { escapeAttribute, escapeHtml, shortId, stateBadge } from "../formatters.js";

const MAX_LOG_KB = 256;

export function renderLogs(state) {
  const availableJobs = state.jobs;
  const selectedJob = availableJobs.find((job) => job.id === state.logs.jobId);
  const query = state.logs.search.trim().toLowerCase();
  const matching = availableJobs.filter((job) => !query || [job.name, job.user, job.id, job.cwd].some((value) => String(value || "").toLowerCase().includes(query)));
  const choices = selectedJob && !matching.some((job) => job.id === selectedJob.id) ? [selectedJob, ...matching] : matching;
  const data = state.logs.data;
  const stream = state.logs.stream === "stderr" ? "stderr" : "stdout";
  const available = data ? Boolean(data[`${stream}_available`]) : false;
  const output = selectedJob && data && available ? data[stream] || "(No output written yet.)" : "Select a job to inspect its output.";
  const truncated = data ? Boolean(data[`${stream}_truncated`]) : false;
  const message = state.logs.error || data?.message || (!availableJobs.length ? "No jobs are available yet. Add a job with the CLI first." : (!available && selectedJob ? `No ${stream} log is available for this job yet.` : ""));
  const options = choices.length ? choices.map((job) => `<option value="${escapeAttribute(job.id)}" ${job.id === state.logs.jobId ? "selected" : ""}>${escapeHtml(job.name)} · ${escapeHtml(shortId(job.id))} · ${escapeHtml(job.state)}</option>`).join("") : '<option value="">No matching jobs</option>';
  return `
    <div class="page-heading"><div><div class="eyebrow">Workspace / Logs</div><h1>Job logs</h1><p>Choose a job, then switch between stdout and stderr. Logs are shown as plain text and are refreshed automatically every 2 seconds.</p></div><button class="button secondary" data-action="refresh">Refresh logs <span aria-hidden="true">↻</span></button></div>
    <section class="panel logs-panel"><div class="logs-toolbar"><label for="log-job-select">Job</label><input class="search-input log-job-search" id="log-job-search" type="search" autocomplete="off" spellcheck="false" value="${escapeAttribute(state.logs.search)}" placeholder="Filter jobs" aria-label="Filter jobs"><select class="select-input log-job-select" id="log-job-select" aria-label="Choose a job"><option value="">Choose a job…</option>${options}</select><div class="log-tabs" role="tablist" aria-label="Log stream"><button class="log-tab ${stream === "stdout" ? "active" : ""}" type="button" data-log-stream="stdout" role="tab" aria-selected="${stream === "stdout"}">stdout</button><button class="log-tab ${stream === "stderr" ? "active" : ""}" type="button" data-log-stream="stderr" role="tab" aria-selected="${stream === "stderr"}">stderr</button></div></div>${selectedJob ? `<div class="log-context"><div><strong>${escapeHtml(selectedJob.name)}</strong><small>${escapeHtml(selectedJob.user)} · ${escapeHtml(selectedJob.cwd)}</small></div>${stateBadge(selectedJob.state)}</div>` : ""}${message ? `<div class="log-message"><span aria-hidden="true">ⓘ</span><span>${escapeHtml(message)}</span></div>` : ""}<pre class="log-output" aria-live="polite">${escapeHtml(output)}</pre>${truncated ? `<div class="log-footnote">Showing the latest ${MAX_LOG_KB} KB of ${stream} output.</div>` : ""}</section>`;
}
