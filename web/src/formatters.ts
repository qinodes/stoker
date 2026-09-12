import type { JobState, Route } from "./types";

export function shortId(id?: string | null): string {
  return id ? `${id.slice(0, 8)}…` : "—";
}

export function routeTitle(route: Route): string {
  return ({ overview: "Overview", jobs: "Jobs", queue: "Queue", logs: "Logs", configuration: "Configuration", policy: "Policy" } as const)[route] || "Overview";
}

export function isTerminalState(value: JobState | string | undefined): boolean {
  return ["SUCCEEDED", "FAILED", "CANCELLED", "LOST"].includes(value || "");
}

export function formatDate(value?: string | Date | null, timezone: string | null = null, locale?: string): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "—";
  const options: Intl.DateTimeFormatOptions = { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" };
  if (timezone) options.timeZone = timezone;
  try {
    return new Intl.DateTimeFormat(locale, options).format(date);
  } catch {
    delete options.timeZone;
    return new Intl.DateTimeFormat(locale, options).format(date);
  }
}

export function limitUnicode(value: string | null | undefined, maximum: number): string {
  return [...String(value ?? "")].slice(0, maximum).join("");
}

export function classForState(value?: JobState | string | null): string {
  return `state-${String(value || "").toLowerCase()}`;
}
