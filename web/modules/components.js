import { escapeAttribute, escapeHtml, liveTime, shortId, stateBadge } from "./formatters.js";

export function pageHeading(eyebrow, title, description, actions = "") {
  return `<div class="page-heading"><div><div class="eyebrow">${escapeHtml(eyebrow)}</div><h1>${escapeHtml(title)}</h1><p>${escapeHtml(description)}</p></div><div class="page-actions">${actions}</div></div>`;
}

export function metric(label, value, note, accent = "") {
  return `<article class="metric-card ${accent}"><div class="metric-label">${escapeHtml(label)}</div><div class="metric-value">${escapeHtml(value)}</div><div class="metric-note">${escapeHtml(note)}</div></article>`;
}

export function emptyState(icon, title, message = "") {
  return `<div class="empty-state"><div class="empty-icon">${icon}</div><strong>${escapeHtml(title)}</strong>${message ? `<p>${escapeHtml(message)}</p>` : ""}</div>`;
}

export function jobRow(job, timezone) {
  return `<tr class="job-row" data-job-open="${escapeAttribute(job.id)}" tabindex="0"><td><div class="job-name"><strong>${escapeHtml(job.name)}</strong><small>${escapeHtml(shortId(job.id))}</small></div></td><td>${escapeHtml(job.user)}</td><td class="path-cell" title="${escapeAttribute(job.cwd)}">${escapeHtml(job.cwd)}</td><td>${stateBadge(job.state)}</td><td class="mono">${job.queue_order ?? "—"}</td><td>${liveTime(job.created_at, timezone)}</td></tr>`;
}

export function pagination(kind, page) {
  if (page.totalItems <= page.items.length && page.page === 1) return "";
  return `<div class="list-pagination"><span>${page.totalItems ? `${page.start + 1}–${page.end} of ${page.totalItems}` : "0 items"}</span><div><button class="button small secondary" data-page-kind="${kind}" data-page="${page.page - 1}" ${page.page <= 1 ? "disabled" : ""}>Previous</button> <button class="button small secondary" data-page-kind="${kind}" data-page="${page.page + 1}" ${page.page >= page.totalPages ? "disabled" : ""}>Next</button></div></div>`;
}

export function toast(message, error = false) {
  const element = document.createElement("div");
  element.className = `toast${error ? " error" : ""}`;
  element.textContent = message;
  return element;
}
