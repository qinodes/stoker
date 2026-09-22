import { LanguagePicker, useI18n } from "./i18n/context";
import { useEffect, useRef, type ReactNode } from "react";
import { useWorkspace } from "./context";
import { routesForMode } from "./modes.ts";
import { ToastRegion } from "./components";
import { Overview } from "./pages/Overview";
import { Jobs, JobDetail, JobForm, DirectoryBrowserDialog } from "./pages/Jobs";
import { Queue } from "./pages/Queue";
import { Logs } from "./pages/Logs";
import { Configuration } from "./pages/Configuration";
import { Policy } from "./pages/Policy";
import { ScheduledOverview } from "./pages/scheduled/Overview";
import { Workloads } from "./pages/scheduled/Workloads";
import { Runs } from "./pages/scheduled/Runs";
import { ScheduledLogs } from "./pages/scheduled/Logs";
import { Sources } from "./pages/scheduled/Sources";
import { ModeChanged } from "./pages/ModeChanged";
import type { Route, WorkspaceMode } from "./types.ts";

export function App() {
  const { t, renderMessage } = useI18n();
  const { state, actions, jobFormOpen, directoryBrowserOpen, jobDetailOpen, detailEditing, confirmation, toasts } = useWorkspace();
  const Page = ({ overview: Overview, jobs: Jobs, queue: Queue, logs: Logs, configuration: Configuration, policy: Policy } as const)[state.route as "overview" | "jobs" | "queue" | "logs" | "configuration" | "policy"];
  const navigation = state.mode ? navigationForMode(state.mode, t) : [];
  const scheduler = state.workspace?.scheduler || state.status?.scheduler;
  const queueLocked = Boolean(state.workspace?.queue_locked ?? state.queue.locked);
  const breadcrumb = navigation.find((item) => item.route === state.route)?.label || (state.modeTransition ? "Mode change" : "Overview");
  return <div className="app-shell"><aside className="sidebar" aria-label={t("nav.primary")}><a className="brand" href="#overview" aria-label={t("nav.overviewLabel")}><span className="brand-mark"><img src="/assets/logo-mark.png" alt={t("brand.mark")} /></span><span className="brand-copy"><strong>stoker</strong><small>{t("brand.tagline")}</small></span></a><div className="workspace-label">{t("common.workspace")}</div><nav className="nav-list">{navigation.map((item) => <NavItem key={item.route} {...item} />)}</nav><div className="sidebar-bottom"><div className="sidebar-mode-control"><WorkspaceModeSwitch /></div><div className="connection-card"><span className={`connection-dot${state.loaded && !state.error ? " online" : state.error ? " error" : ""}`} id="connection-dot"></span><div><strong id="connection-label">{state.loaded && !state.error ? t("connection.connected") : state.error ? t("connection.issue") : t("connection.connecting")}</strong><small id="connection-detail">{state.loaded && !state.error ? t("connection.synced") : state.error ? t("connection.retry") : t("connection.reading")}</small></div></div><div className="version-label">Stoker <span id="app-version">{state.config?.version || "—"}</span></div></div></aside>
    <main className="main-content" data-view={state.route}><header className="topbar"><div className="breadcrumbs"><span>{t("common.workspace")}</span><span className="crumb-separator">/</span><strong id="breadcrumb-current">{breadcrumb}</strong></div><div className="topbar-actions"><div className="topbar-mode-control"><WorkspaceModeSwitch /></div><button className={`button small workspace-lock-control ${queueLocked ? "locked" : "unlocked"}`} data-action="workspace-queue-lock" data-queue-lock-state={queueLocked ? "locked" : "unlocked"} type="button" onClick={() => void actions.queueLock(!queueLocked)}><span className="status-dot" aria-hidden="true"></span><span>{queueLocked ? t("queue.unlock") : t("queue.lock")}</span></button><span className={`scheduler-pill ${scheduler?.running ? "running" : "stopped"}`} id="scheduler-pill"><span className="status-dot"></span><span id="scheduler-label">{scheduler?.running ? t("scheduler.running") : t("scheduler.stopped")}</span></span><LanguagePicker /></div></header><section className="page" id="app" data-view={state.route} aria-live="polite">{state.modeTransition ? <ModeChanged /> : !state.loaded ? (state.error ? <div className="page-heading"><div><div className="eyebrow">{t("workspace.unavailable")}</div><h1>{t("workspace.loadFailed")}</h1><p>{state.error}</p></div><button className="button primary" data-action="retry" onClick={() => void actions.loadData()}>{t("common.retry")}</button></div> : <div className="page-loading"><span className="spinner"></span><span>{t("workspace.loading")}</span></div>) : state.mode === "scheduled" ? <ScheduledPage /> : <Page />}</section></main>
    <ToastRegion toasts={toasts} />
    <ModalDialog id="confirm-dialog" className="confirm-dialog" open={Boolean(confirmation)} onClose={() => actions.resolveConfirmation(false)}><div className="dialog-card confirm-card"><div className="dialog-kicker" id="confirm-kicker">{confirmation ? renderMessage(confirmation.kicker) : t("nav.configuration")}</div><h2 id="confirm-title">{confirmation ? renderMessage(confirmation.title) : t("common.confirmAction")}</h2><p id="confirm-message">{renderMessage(confirmation?.message || "")}</p><div className="dialog-actions"><button className="button secondary" id="confirm-cancel" type="button" onClick={() => actions.resolveConfirmation(false)}>{t("common.cancel")}</button><button className={`button ${confirmation?.destructive ? "danger" : "primary"}`} id="confirm-accept" type="button" onClick={() => actions.resolveConfirmation(true)}>{confirmation ? renderMessage(confirmation.acceptLabel) : t("common.confirm")}</button></div></div></ModalDialog>
    <ModalDialog id="job-dialog" className="job-dialog" open={jobFormOpen} onClose={actions.closeJobForm}><div className="job-dialog-shell" id="job-dialog-content"><JobForm /></div></ModalDialog>
    <ModalDialog id="directory-browser-dialog" className="directory-browser-dialog" open={directoryBrowserOpen} onClose={actions.closeDirectoryBrowser}><DirectoryBrowserDialog /></ModalDialog>
    <ModalDialog id="job-detail-dialog" className="job-detail-dialog" open={jobDetailOpen} onClose={actions.closeJobDetail}><div className="job-detail-shell" id="job-detail-content">{detailEditing || state.selectedJob ? <JobDetail /> : <div className="page-loading"><span className="spinner"></span><span>{t("jobs.loading")}</span></div>}</div></ModalDialog>
  </div>;
}

function navigationForMode(mode: "serial" | "scheduled", t: (key: any) => string) {
  const icons: Record<Exclude<Route, "mode-change">, string> = { overview: "◈", jobs: "▤", queue: "≡", workloads: "▤", runs: "◷", logs: "⌁", sources: "⌘", configuration: "◌", policy: "⚙" };
  const labels: Record<Exclude<Route, "mode-change">, string> = { overview: t("nav.overview"), jobs: t("nav.jobs"), queue: t("nav.queue"), workloads: t("scheduled.nav.workloads"), runs: t("scheduled.nav.runs"), logs: t("nav.logs"), sources: t("scheduled.nav.sources"), configuration: t("nav.configuration"), policy: t("nav.policy") };
  return routesForMode(mode).map((route) => ({ route, icon: icons[route], label: labels[route] }));
}

function NavItem({ route, icon, label }: { route: Exclude<Route, "mode-change">; icon: string; label: string }) {
  const { state } = useWorkspace();
  return <a className={`nav-item${state.route === route ? " active" : ""}`} href={`#${route}`} data-route={route}><span className="nav-icon">{icon}</span><span className="nav-label">{label}</span></a>;
}

function WorkspaceModeSwitch() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const mode: WorkspaceMode = state.mode ?? state.modeTransition?.actualMode ?? "serial";
  const pending = Boolean(state.modeTransition);
  const disabled = !state.mode || pending;
  return <div className={`workspace-mode-switch${mode === "scheduled" ? " scheduled" : ""}${pending ? " pending" : ""}`} data-workspace-mode-switch data-workspace-mode={mode} role="radiogroup" aria-label={t("workspace.mode")} aria-busy={pending}>
    <span className="workspace-mode-thumb" aria-hidden="true"></span>
    <button className="workspace-mode-option" data-workspace-mode-option="serial" type="button" role="radio" aria-checked={mode === "serial"} disabled={disabled} onClick={() => void actions.setWorkspaceMode("serial")}>{t("workspace.mode.serial")}</button>
    <button className="workspace-mode-option" data-workspace-mode-option="scheduled" type="button" role="radio" aria-checked={mode === "scheduled"} disabled={disabled} onClick={() => void actions.setWorkspaceMode("scheduled")}>{t("workspace.mode.scheduled")}</button>
  </div>;
}

function ScheduledPage() {
  const { state } = useWorkspace();
  const { t } = useI18n();
  if (state.route === "overview") return <ScheduledOverview />;
  if (state.route === "workloads") return <Workloads />;
  if (state.route === "runs") return <Runs />;
  if (state.route === "logs") return <ScheduledLogs />;
  if (state.route === "sources") return <Sources />;
  if (state.route === "configuration") return <Configuration />;
  if (state.route === "policy") return <Policy />;
  return <div className="page-heading"><div><div className="eyebrow">{t("scheduled.page.eyebrow")}</div><h1>{state.route}</h1><p>{t("scheduled.page.unavailable")}</p></div></div>;
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
