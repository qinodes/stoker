import { useI18n } from "./i18n/context";
import { useEffect, useState, type ReactNode, type KeyboardEvent } from "react";
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
  const [activeIndex, setActiveIndex] = useState(-1);
  const query = value.trim().toLowerCase();
  const suggestions = query ? timezones.filter((zone) => zone.toLowerCase().includes(query)).slice(0, 8) : [];
  const visible = showSuggestions && suggestions.length > 0;
  const choose = (zone: string) => { onChange(zone); setShowSuggestions(false); setActiveIndex(-1); };
  useEffect(() => {
    if (activeIndex >= 0) document.getElementById(`${suggestionsId}-option-${activeIndex}`)?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, suggestionsId]);
  return <div className="timezone-picker"><input className="text-input" id={id} name="timezone" value={value} placeholder={placeholder} required={required} autoComplete="off" role="combobox" aria-autocomplete="list" aria-controls={suggestionsId} aria-expanded={visible} aria-activedescendant={visible && activeIndex >= 0 ? `${suggestionsId}-option-${activeIndex}` : undefined} onFocus={() => { setShowSuggestions(true); setActiveIndex(-1); }} onChange={(event) => { onChange(event.target.value); setShowSuggestions(true); setActiveIndex(-1); }} onKeyDown={(event) => {
    if ((event.key === "ArrowDown" || event.key === "ArrowUp") && suggestions.length) {
      event.preventDefault();
      setShowSuggestions(true);
      setActiveIndex((current) => event.key === "ArrowDown" ? Math.min(current + 1, suggestions.length - 1) : current <= 0 ? suggestions.length - 1 : current - 1);
    } else if (event.key === "Enter" && visible && activeIndex >= 0) {
      event.preventDefault();
      choose(suggestions[activeIndex]);
    } else if (event.key === "Escape" && showSuggestions) {
      event.preventDefault();
      setShowSuggestions(false);
      setActiveIndex(-1);
    }
  }} /><div className={`timezone-suggestions${visible ? " visible" : ""}`} id={suggestionsId} role="listbox">{suggestions.map((zone, index) => <button className={`timezone-option${index === activeIndex ? " active" : ""}`} id={`${suggestionsId}-option-${index}`} type="button" role="option" aria-selected={index === activeIndex} data-timezone={zone} key={zone} onMouseEnter={() => setActiveIndex(index)} onClick={() => choose(zone)}>{zone}</button>)}</div></div>;
}

const HOURS = Array.from({ length: 24 }, (_, index) => String(index).padStart(2, "0"));
const MINUTES = Array.from({ length: 60 }, (_, index) => String(index).padStart(2, "0"));

export function TimePicker({ id, value, hourLabel, minuteLabel, required = false, onChange }: { id: string; value: string; hourLabel: string; minuteLabel: string; required?: boolean; onChange: (value: string) => void }) {
  const match = /^(\d{2}):(\d{2})$/.exec(value);
  const hour = match?.[1] || "";
  const minute = match?.[2] || "";
  return <div className="time-picker" id={id}><select className="select-input" aria-label={hourLabel} required={required} value={hour} onChange={(event) => onChange(`${event.target.value}:${minute || "00"}`)}><option value="" disabled>HH</option>{HOURS.map((item) => <option value={item} key={item}>{item}</option>)}</select><span className="time-picker-separator" aria-hidden="true">:</span><select className="select-input" aria-label={minuteLabel} required={required} value={minute} onChange={(event) => onChange(`${hour || "00"}:${event.target.value}`)}><option value="" disabled>MM</option>{MINUTES.map((item) => <option value={item} key={item}>{item}</option>)}</select></div>;
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

export function Pagination({ kind, page, onPage }: { kind: "jobs" | "flows" | "snapshots"; page: PageInfo<unknown>; onPage: (page: number) => void }) {
  const { t } = useI18n();
  if (page.totalPages <= 1 && kind === "snapshots") return null;
  const from = page.totalItems ? page.start + 1 : 0;
  const label = kind === "jobs" ? t("pagination.jobs") : kind === "flows" ? t("pagination.flows") : t("pagination.snapshots");
  return <nav className="list-pagination" aria-label={label}><span>{t("pagination.showing", { from, end: page.end, total: page.totalItems })}</span><div className="list-pagination-controls">
    <button className="button small secondary" type="button" disabled={page.page === 1} onClick={() => onPage(page.page - 1)}>{t("common.previous")}</button>
    <span>{t("pagination.page", { page: page.page, total: page.totalPages })}</span>
    <button className="button small secondary" type="button" disabled={page.page === page.totalPages} onClick={() => onPage(page.page + 1)}>{t("common.next")}</button>
  </div></nav>;
}

export function ToastRegion({ toasts }: { toasts: Array<{ id: number; message: UiMessage; error: boolean }> }) {
  const { renderMessage } = useI18n();
  return <div className="toast-region" id="toast-region" aria-live="assertive" aria-atomic="true">{toasts.map((toast) => <div key={toast.id} className={`toast${toast.error ? " error" : ""}`}>{renderMessage(toast.message)}</div>)}</div>;
}
