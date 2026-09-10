import { useEffect, useRef, type ReactNode } from "react";
import { useWorkspace } from "./context";
import { routeTitle } from "./formatters";
import { ToastRegion } from "./components";
import { Overview } from "./pages/Overview";
import { Jobs, JobDetail, JobForm } from "./pages/Jobs";
import { Queue } from "./pages/Queue";
import { Logs } from "./pages/Logs";
import { Configuration } from "./pages/Configuration";

export function App() {
  const { state, actions, jobFormOpen, jobDetailOpen, detailEditing, confirmation, toasts } = useWorkspace();
  const Page = ({ overview: Overview, jobs: Jobs, queue: Queue, logs: Logs, configuration: Configuration } as const)[state.route];
  return <div className="app-shell"><aside className="sidebar" aria-label="Primary navigation"><a className="brand" href="#overview" aria-label="Stoker overview"><span className="brand-mark"><img src="/assets/logo-mark.png" alt="Stoker flame mark" /></span><span className="brand-copy"><strong>stoker</strong><small>execution forge</small></span></a><div className="workspace-label">Workspace</div><nav className="nav-list"><NavItem route="overview" icon="◈" label="Overview" /><NavItem route="jobs" icon="▤" label="Jobs" /><NavItem route="queue" icon="≡" label="Queue" /><NavItem route="logs" icon="⌁" label="Logs" /><NavItem route="configuration" icon="◌" label="Configuration" /></nav><div className="sidebar-bottom"><div className="connection-card"><span className={`connection-dot${state.loaded && !state.error ? " online" : state.error ? " error" : ""}`} id="connection-dot"></span><div><strong id="connection-label">{state.loaded && !state.error ? "Server connected" : state.error ? "Connection issue" : "Connecting"}</strong><small id="connection-detail">{state.loaded && !state.error ? "State synced just now" : state.error ? "Retry to reconnect" : "Reading workspace state"}</small></div></div><div className="version-label">Stoker <span id="app-version">{state.config?.version || "—"}</span></div></div></aside>
    <main className="main-content" data-view={state.route}><header className="topbar"><div className="breadcrumbs"><span>Workspace</span><span className="crumb-separator">/</span><strong id="breadcrumb-current">{routeTitle(state.route)}</strong></div><div className="topbar-actions"><span className={`scheduler-pill ${state.status?.scheduler.running ? "running" : "stopped"}`} id="scheduler-pill"><span className="status-dot"></span><span id="scheduler-label">{state.status?.scheduler.running ? "Scheduler running" : "Scheduler stopped"}</span></span><button className="icon-button" id="refresh-button" type="button" title="Refresh data" aria-label="Refresh data" onClick={() => void actions.loadData()}>↻</button><button className="avatar-button" type="button" title="Local UI session" aria-label="Local UI session">S</button></div></header><section className="page" id="app" data-view={state.route} aria-live="polite">{!state.loaded ? (state.error ? <div className="page-heading"><div><div className="eyebrow">Workspace unavailable</div><h1>Couldn’t load Stoker</h1><p>{state.error}</p></div><button className="button primary" data-action="refresh" onClick={() => void actions.loadData()}>Try again</button></div> : <div className="page-loading"><span className="spinner"></span><span>Loading workspace…</span></div>) : <Page />}</section></main>
    <ToastRegion toasts={toasts} />
    <ModalDialog id="confirm-dialog" className="confirm-dialog" open={Boolean(confirmation)} onClose={() => actions.resolveConfirmation(false)}><div className="dialog-card confirm-card"><div className="dialog-kicker" id="confirm-kicker">{confirmation?.kicker || "Configuration"}</div><h2 id="confirm-title">{confirmation?.title || "Confirm action"}</h2><p id="confirm-message">{confirmation?.message || ""}</p><div className="dialog-actions"><button className="button secondary" id="confirm-cancel" type="button" onClick={() => actions.resolveConfirmation(false)}>Cancel</button><button className={`button ${confirmation?.destructive ? "danger" : "primary"}`} id="confirm-accept" type="button" onClick={() => actions.resolveConfirmation(true)}>{confirmation?.acceptLabel || "Confirm"}</button></div></div></ModalDialog>
    <ModalDialog id="job-dialog" className="job-dialog" open={jobFormOpen} onClose={actions.closeJobForm}><div className="job-dialog-shell" id="job-dialog-content"><JobForm /></div></ModalDialog>
    <ModalDialog id="job-detail-dialog" className="job-detail-dialog" open={jobDetailOpen} onClose={actions.closeJobDetail}><div className="job-detail-shell" id="job-detail-content">{detailEditing || state.selectedJob ? <JobDetail /> : <div className="page-loading"><span className="spinner"></span><span>Loading job…</span></div>}</div></ModalDialog>
  </div>;
}

function NavItem({ route, icon, label }: { route: "overview" | "jobs" | "queue" | "logs" | "configuration"; icon: string; label: string }) {
  const { state } = useWorkspace();
  return <a className={`nav-item${state.route === route ? " active" : ""}`} href={`#${route}`} data-route={route}><span className="nav-icon">{icon}</span>{label}</a>;
}

function ModalDialog({ id, className, open, onClose, children }: { id: string; className: string; open: boolean; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);
  return <dialog ref={ref} id={id} className={className} onCancel={(event) => { event.preventDefault(); onClose(); }} onClick={(event) => { if (event.target === event.currentTarget) onClose(); }}>{children}</dialog>;
}
