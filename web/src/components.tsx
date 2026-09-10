import type { ReactNode, KeyboardEvent } from "react";
import type { Job, PageInfo } from "./types";
import { classForState, formatDate, shortId } from "./formatters";

export function LiveTime({ value, timezone }: { value?: string | Date | null; timezone?: string | null }) {
  if (!value) return <>—</>;
  const serialized = value instanceof Date ? value.toISOString() : value;
  return <time className="live-time" dateTime={serialized}>{formatDate(serialized, timezone)}</time>;
}

export function StateBadge({ value }: { value?: string | null }) {
  return <span className={`state-badge ${classForState(value)}`}>{value || "UNKNOWN"}</span>;
}

export function PageHeading({ eyebrow, title, description, actions }: { eyebrow: string; title: string; description: string; actions?: ReactNode }) {
  return <div className="page-heading"><div><div className="eyebrow">{eyebrow}</div><h1>{title}</h1><p>{description}</p></div><div className="page-actions">{actions}</div></div>;
}

export function Metric({ label, value, note, accent = "" }: { label: string; value: string | number; note: string; accent?: string }) {
  return <article className={`metric-card ${accent}`}><div className="metric-label">{label}</div><div className="metric-value">{value}</div><div className="metric-note">{note}</div></article>;
}

export function EmptyState({ icon, title, message, compact = false }: { icon?: ReactNode; title: string; message?: string; compact?: boolean }) {
  return <div className={`empty-state${compact ? " compact" : ""}`}>{icon !== undefined && <div className="empty-icon">{icon}</div>}<strong>{title}</strong>{message && <p>{message}</p>}</div>;
}

export function JobRow({ job, timezone, onOpen }: { job: Job; timezone?: string | null; onOpen: (id: string) => void }) {
  const open = () => onOpen(job.id);
  const keyboard = (event: KeyboardEvent<HTMLTableRowElement>) => {
    if (event.key === "Enter" || event.key === " ") { event.preventDefault(); open(); }
  };
  return <tr className="job-row" data-job-open={job.id} tabIndex={0} aria-label={`Open details for ${job.name}`} onClick={open} onKeyDown={keyboard}>
    <td><div className="job-name"><strong>{job.name}</strong><small>{shortId(job.id)}</small></div></td>
    <td>{job.user}</td>
    <td className="path-cell" title={job.cwd}>{job.cwd}</td>
    <td><StateBadge value={job.state} /></td>
    <td className="mono">{job.queue_order ?? "—"}</td>
    <td><LiveTime value={job.created_at} timezone={timezone} /></td>
  </tr>;
}

export function Pagination({ kind, page, onPage }: { kind: "jobs" | "snapshots"; page: PageInfo<unknown>; onPage: (page: number) => void }) {
  if (page.totalPages <= 1 && kind !== "jobs") return null;
  const from = page.totalItems ? page.start + 1 : 0;
  return <nav className="list-pagination" aria-label={`${kind} pagination`}><span>Showing {from}–{page.end} of {page.totalItems}</span><div className="list-pagination-controls">
    <button className="button small secondary" type="button" disabled={page.page === 1} onClick={() => onPage(page.page - 1)}>Previous</button>
    <span>Page {page.page} of {page.totalPages}</span>
    <button className="button small secondary" type="button" disabled={page.page === page.totalPages} onClick={() => onPage(page.page + 1)}>Next</button>
  </div></nav>;
}

export function ToastRegion({ toasts }: { toasts: Array<{ id: number; message: string; error: boolean }> }) {
  return <div className="toast-region" id="toast-region" aria-live="assertive" aria-atomic="true">{toasts.map((toast) => <div key={toast.id} className={`toast${toast.error ? " error" : ""}`}>{toast.message}</div>)}</div>;
}
