import { emptyState, pageHeading } from "../components.js";
import { escapeHtml, shortId, stateBadge } from "../formatters.js";

export function renderQueue(state) {
  const jobs = state.queue.jobs || [];
  const toggle = state.queue.locked
    ? '<button class="button secondary" data-queue-lock="false">Unlock queue</button>'
    : '<button class="button primary" data-queue-lock="true">Lock queue</button>';
  const rows = jobs.map((job, index) => `<tr><td class="mono">${index + 1}</td><td><div class="job-name"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(shortId(job.id))}</small></div></td><td>${escapeHtml(job.user)}</td><td>${stateBadge(job.state)}</td><td><div class="move-controls"><button class="move-button" data-queue-move="${job.id}" data-target-order="${Math.max(1, index)}" ${!state.queue.locked || index === 0 ? "disabled" : ""}>↑</button><button class="move-button" data-queue-move="${job.id}" data-target-order="${index + 2}" ${!state.queue.locked || index === jobs.length - 1 ? "disabled" : ""}>↓</button></div></td></tr>`).join("");
  return `${pageHeading("Workspace / Queue", "Execution queue", "Lock the queue before changing the next execution order.", toggle)}<section class="panel"><div class="${state.queue.locked ? "queue-lock-banner" : "queue-unlock-banner"}"><div><strong>${state.queue.locked ? "Queue is locked" : "Queue is unlocked"}</strong><small>${state.queue.locked ? "Reordering is available." : "Lock the queue to reorder jobs."}</small></div></div><div class="queue-table table-wrap">${rows ? `<table class="queue-order-table"><thead><tr><th>Order</th><th>Job</th><th>Owner</th><th>State</th><th>Move</th></tr></thead><tbody>${rows}</tbody></table>` : emptyState("≡", "Queue is clear", "Commit a draft job to add it here.")}</div></section>`;
}
