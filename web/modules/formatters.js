export function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    "'": "&#39;",
    '"': "&quot;",
  })[character]);
}

export const escapeAttribute = escapeHtml;

export function limitUnicode(value, maximum) {
  return [...String(value ?? "")].slice(0, maximum).join("");
}

export function shortId(id) {
  return id ? `${String(id).slice(0, 8)}…` : "—";
}

export function routeTitle(route) {
  return ({
    overview: "Overview",
    jobs: "Jobs",
    queue: "Queue",
    logs: "Logs",
    configuration: "Configuration",
  })[route] || "Overview";
}

export function isTerminalState(value) {
  return ["SUCCEEDED", "FAILED", "CANCELLED", "LOST"].includes(value);
}

export function formatDate(value, timezone = null, locale = undefined) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "—";
  const options = { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" };
  if (timezone) options.timeZone = timezone;
  try {
    return new Intl.DateTimeFormat(locale, options).format(date);
  } catch {
    delete options.timeZone;
    return new Intl.DateTimeFormat(locale, options).format(date);
  }
}

export function liveTime(value, timezone = null) {
  if (!value) return "—";
  const serialized = value instanceof Date ? value.toISOString() : String(value);
  return `<time class="live-time" datetime="${escapeAttribute(serialized)}">${formatDate(serialized, timezone)}</time>`;
}

export function stateBadge(value) {
  const key = String(value || "").toLowerCase();
  return `<span class="state-badge state-${key}">${escapeHtml(value || "UNKNOWN")}</span>`;
}
