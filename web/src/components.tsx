import { useI18n } from "./i18n/context";
import { useState, type ReactNode, type KeyboardEvent } from "react";
import type { UiMessage } from "./i18n/messages.ts";
import type { Job, PageInfo } from "./types";
import { classForState, formatDate, shortId } from "./formatters";

export function LiveTime({ value, timezone }: { value?: string | Date | null; timezone?: string | null }) {
  const { locale } = useI18n();
  if (!value) return <>—</>;
  const serialized = value instanceof Date ? value.toISOString() : value;
  return <time className="live-time" dateTime={serialized}>{formatDate(serialized, timezone, locale)}</time>;
}

export function StateBadge({ value }: { value?: string | null }) {
  const { stateLabel } = useI18n();
  return <span className={`state-badge ${classForState(value)}`}><span className="state-badge-label">{stateLabel(value)}</span></span>;
}

export function PageHeading({ eyebrow, title, description, actions }: { eyebrow: string; title: string; description: string; actions?: ReactNode }) {
  return <div className="page-heading"><div><div className="eyebrow">{eyebrow}</div><h1>{title}</h1><p>{description}</p></div><div className="page-actions">{actions}</div></div>;
}

export function TimezonePicker({ id, suggestionsId, value, timezones, placeholder, required = false, onChange }: { id: string; suggestionsId: string; value: string; timezones: string[]; placeholder: string; required?: boolean; onChange: (value: string) => void }) {
  const [showSuggestions, setShowSuggestions] = useState(false);
  const query = value.trim().toLowerCase();
  const suggestions = query ? timezones.filter((zone) => zone.toLowerCase().includes(query)).slice(0, 8) : [];
  return <div className="timezone-picker"><input className="text-input" id={id} name="timezone" value={value} placeholder={placeholder} required={required} autoComplete="off" role="combobox" aria-autocomplete="list" aria-controls={suggestionsId} aria-expanded={showSuggestions && suggestions.length > 0} onFocus={() => setShowSuggestions(true)} onChange={(event) => { onChange(event.target.value); setShowSuggestions(true); }} /><div className={`timezone-suggestions${showSuggestions && suggestions.length ? " visible" : ""}`} id={suggestionsId} role="listbox">{suggestions.map((zone) => <button className="timezone-option" type="button" role="option" data-timezone={zone} key={zone} onClick={() => { onChange(zone); setShowSuggestions(false); }}>{zone}</button>)}</div></div>;
}

export function Metric({ label, value, note, accent = "" }: { label: string; value: string | number; note: string; accent?: string }) {
  return <article className={`metric-card ${accent}`}><div className="metric-label">{label}</div><div className="metric-value">{value}</div><div className="metric-note">{note}</div></article>;
}

export function EmptyState({ icon, title, message, compact = false, actions }: { icon?: ReactNode; title: string; message?: string; compact?: boolean; actions?: ReactNode }) {
  return <div className={`empty-state${compact ? " compact" : ""}`}>{icon !== undefined && <div className="empty-icon">{icon}</div>}<strong>{title}</strong>{message && <p>{message}</p>}{actions && <div className="page-actions">{actions}</div>}</div>;
}

export function JobRow({ job, timezone, onOpen }: { job: Job; timezone?: string | null; onOpen: (id: string) => void }) {
  const { t } = useI18n();
  const open = () => onOpen(job.id);
  const keyboard = (event: KeyboardEvent<HTMLTableRowElement>) => {
    if (event.key === "Enter" || event.key === " ") { event.preventDefault(); open(); }
  };
  return <tr className="job-row" data-job-open={job.id} tabIndex={0} aria-label={t("jobs.openDetails", { name: job.name })} onClick={open} onKeyDown={keyboard}>
    <td><div className="job-name"><strong>{job.name}</strong><small>{shortId(job.id)}</small></div></td>
    <td>{job.user}</td>
    <td className="path-cell" title={job.cwd}>{job.cwd}</td>
    <td><StateBadge value={job.state} /></td>
    <td className="mono">{job.queue_order ?? "—"}</td>
    <td><LiveTime value={job.created_at} timezone={timezone} /></td>
  </tr>;
}

export function Pagination({ kind, page, onPage }: { kind: "jobs" | "snapshots"; page: PageInfo<unknown>; onPage: (page: number) => void }) {
  const { t } = useI18n();
  if (page.totalPages <= 1 && kind !== "jobs") return null;
  const from = page.totalItems ? page.start + 1 : 0;
  return <nav className="list-pagination" aria-label={t(kind === "jobs" ? "pagination.jobs" : "pagination.snapshots")}><span>{t("pagination.showing", { from, end: page.end, total: page.totalItems })}</span><div className="list-pagination-controls">
    <button className="button small secondary" type="button" disabled={page.page === 1} onClick={() => onPage(page.page - 1)}>{t("common.previous")}</button>
    <span>{t("pagination.page", { page: page.page, total: page.totalPages })}</span>
    <button className="button small secondary" type="button" disabled={page.page === page.totalPages} onClick={() => onPage(page.page + 1)}>{t("common.next")}</button>
  </div></nav>;
}

export function ToastRegion({ toasts }: { toasts: Array<{ id: number; message: UiMessage; error: boolean }> }) {
  const { renderMessage } = useI18n();
  return <div className="toast-region" id="toast-region" aria-live="assertive" aria-atomic="true">{toasts.map((toast) => <div key={toast.id} className={`toast${toast.error ? " error" : ""}`}>{renderMessage(toast.message)}</div>)}</div>;
}
