import { metric } from "../components.js";
import { escapeHtml, liveTime, shortId, stateBadge } from "../formatters.js";

export function renderOverview(state) {
  const counts = state.status.counts;
  const active = state.jobs.find((job) => ["STARTING", "RUNNING", "CANCELLING"].includes(job.state));
  const queue = state.queue.jobs.slice(0, 5);
  return `
    <div class="page-heading">
      <div><div class="eyebrow">Queue control center</div><h1>See what’s running and what’s next.</h1><p>Keep long-running work moving with a clear view of what is running, waiting, and ready to ship.</p></div>
      <button class="button secondary" data-action="refresh">Refresh workspace <span aria-hidden="true">↻</span></button>
    </div>
    <div class="metrics-grid">
      ${metric("Running", counts.active, active ? active.name : "No active job", "accent-ember")}
      ${metric("Queued", counts.queued, state.queue.locked ? "Queue is locked" : "Ready to run", "accent-cyan")}
      ${metric("Drafts", counts.draft, "Awaiting review", "")}
      ${metric("Succeeded", counts.succeeded, "Completed jobs", "accent-green")}
      ${metric("Failed / lost", counts.failed, "Needs attention", "")}
    </div>
    <div class="content-grid">
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>Active job</h2><p>What the scheduler is working on right now</p></div></div><a class="panel-link" href="#jobs">View all jobs →</a></div><div class="active-job">${activeJobMarkup(active, state)}</div></section>
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>Up next</h2><p>Queue execution order</p></div></div><a class="panel-link" href="#queue">Manage queue →</a></div><div class="queue-list">${queueMarkup(queue)}</div></section>
      <section class="panel"><div class="panel-header"><div class="panel-title"><div><h2>Recent activity</h2><p>Latest persisted job events</p></div></div><a class="panel-link" href="#jobs">Open jobs →</a></div><div class="activity-list">${activityMarkup(state.jobs, state)}</div></section>
      <section class="panel"><div class="notice-panel"><div class="notice-icon">⌘</div><div><div class="section-kicker">Terminal workflow</div><h2>${state.status.scheduler.running ? "Scheduler is online" : "Scheduler is stopped"}</h2><p>${state.status.scheduler.running ? "Newly committed jobs can be claimed in queue order." : "Run <code>stoker start</code> in the terminal to process queued work. The browser UI remains available for inspection."}</p></div></div></section>
    </div>`;
}

function activeJobMarkup(job, state) {
  if (!job) return '<div class="empty-state"><div class="empty-icon">○</div><strong>No active job</strong><p>When the scheduler claims work, the current command and owner will appear here.</p></div>';
  return `<div class="job-hero" data-job-open="${job.id}"><div><h3>${escapeHtml(job.name)}</h3><p>${escapeHtml(job.cwd)}</p><div class="job-hero-meta"><span>${escapeHtml(job.user)}</span><span>${escapeHtml(shortId(job.id))}</span><span>${liveTime(job.started_at || job.created_at, state.timezone?.name)}</span></div></div>${stateBadge(job.state)}</div>`;
}

function queueMarkup(jobs) {
  if (!jobs.length) return '<div class="empty-state"><div class="empty-icon">≡</div><strong>Queue is clear</strong><p>Committed jobs will show up here in execution order.</p></div>';
  return jobs.map((job, index) => `<div class="queue-row" data-job-open="${job.id}"><span class="queue-order">${String(index + 1).padStart(2, "0")}</span><div class="queue-job"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(job.user)} · ${escapeHtml(shortId(job.id))}</small></div><span class="queue-state">QUEUED</span></div>`).join("");
}

function activityMarkup(jobs, state) {
  const recent = [...jobs].sort((a, b) => new Date(b.finished_at || b.created_at) - new Date(a.finished_at || a.created_at)).slice(0, 4);
  if (!recent.length) return '<div class="empty-state"><div class="empty-icon">⌁</div><strong>No activity yet</strong><p>Create a DRAFT job with the CLI to start building your workspace.</p></div>';
  return recent.map((job) => `<div class="activity-item" data-job-open="${job.id}"><div class="activity-line"></div><div class="activity-copy"><strong>${escapeHtml(job.name)} · ${escapeHtml(job.state)}</strong><small>${escapeHtml(job.user)} · ${liveTime(job.finished_at || job.created_at, state.timezone?.name)}</small></div></div>`).join("");
}
