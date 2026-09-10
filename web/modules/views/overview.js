import { emptyState, metric, pageHeading } from "../components.js";
import { escapeHtml, liveTime, shortId, stateBadge } from "../formatters.js";

export function renderOverview(state) {
  const counts = state.status.counts;
  const active = state.jobs.find((job) => ["STARTING", "RUNNING", "CANCELLING"].includes(job.state));
  const queued = state.queue.jobs || [];
  const recent = [...state.jobs]
    .sort((left, right) => new Date(right.finished_at || right.created_at) - new Date(left.finished_at || left.created_at))
    .slice(0, 4);
  return `${pageHeading("Workspace / Overview", "Execution at a glance", "Live scheduler state, queue pressure, and recent job outcomes.", '<button class="button primary" data-action="new-job">New job <span>＋</span></button>')}
    <div class="metrics-grid">
      ${metric("Total jobs", counts.total, "All recorded work", "accent-ember")}
      ${metric("Queued", counts.queued, state.queue.locked ? "Queue locked" : "Ready to dispatch", "accent-cyan")}
      ${metric("Active", counts.active, state.status.scheduler.running ? "Scheduler online" : "Scheduler offline")}
      ${metric("Succeeded", counts.succeeded, "Completed cleanly", "accent-green")}
      ${metric("Failed", counts.failed, "Failed or lost")}
    </div>
    <div class="content-grid">
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>Active job</h2><p>Current process ownership</p></div></div></div><div class="active-job">${active ? activeMarkup(active, state) : emptyState("○", "No active job", "The scheduler has not claimed work.")}</div></section>
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>Queue</h2><p>${queued.length} job${queued.length === 1 ? "" : "s"} waiting</p></div></div><a class="button small ghost" href="#queue">Manage</a></div><div class="queue-list">${queued.length ? queued.slice(0, 5).map(queueRow).join("") : emptyState("≡", "Queue is clear", "Committed jobs appear here.")}</div></section>
      <section class="panel activity-panel"><div class="panel-header"><div class="panel-title"><div><h2>Recent activity</h2><p>Latest state changes</p></div></div></div><div class="activity-list">${recent.length ? recent.map((job) => activity(job, state)).join("") : emptyState("⌁", "No activity yet")}</div></section>
    </div>`;
}

function activeMarkup(job, state) {
  return `<div class="job-hero" data-job-open="${job.id}"><div><h3>${escapeHtml(job.name)}</h3><p>${escapeHtml(job.cwd)}</p><div class="job-hero-meta"><span>${escapeHtml(job.user)}</span><span>${escapeHtml(shortId(job.id))}</span><span>${liveTime(job.started_at || job.created_at, state.timezone?.name)}</span></div></div>${stateBadge(job.state)}</div>`;
}

function queueRow(job, index) {
  return `<div class="queue-row" data-job-open="${job.id}"><span class="queue-order">${String(index + 1).padStart(2, "0")}</span><div class="queue-job"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(job.user)} · ${escapeHtml(shortId(job.id))}</small></div><span class="queue-state">QUEUED</span></div>`;
}

function activity(job, state) {
  return `<div class="activity-item" data-job-open="${job.id}"><div class="activity-line"></div><div class="activity-copy"><strong>${escapeHtml(job.name)} · ${escapeHtml(job.state)}</strong><small>${escapeHtml(job.user)} · ${liveTime(job.finished_at || job.created_at, state.timezone?.name)}</small></div></div>`;
}
