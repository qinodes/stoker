import { LanguagePicker, useI18n } from "./i18n/context";
import { useEffect, useRef, type ReactNode } from "react";
import { useWorkspace } from "./context";
import { ToastRegion } from "./components";
import { Overview } from "./pages/Overview";
import { Jobs, JobDetail, JobForm } from "./pages/Jobs";
import { Queue } from "./pages/Queue";
import { Logs } from "./pages/Logs";
import { Configuration } from "./pages/Configuration";
import { Policy } from "./pages/Policy";

export function App() {
  const { t, renderMessage } = useI18n();
  const { state, actions, jobFormOpen, jobDetailOpen, detailEditing, confirmation, toasts } = useWorkspace();
  const Page = ({ overview: Overview, jobs: Jobs, queue: Queue, logs: Logs, configuration: Configuration, policy: Policy } as const)[state.route];
  return <div className="app-shell"><aside className="sidebar" aria-label={t("nav.primary")}><a className="brand" href="#overview" aria-label={t("nav.overviewLabel")}><span className="brand-mark"><img src="/assets/logo-mark.png" alt={t("brand.mark")} /></span><span className="brand-copy"><strong>stoker</strong><small>{t("brand.tagline")}</small></span></a><div className="workspace-label">{t("common.workspace")}</div><nav className="nav-list"><NavItem route="overview" icon="◈" label={t("nav.overview")} /><NavItem route="jobs" icon="▤" label={t("nav.jobs")} /><NavItem route="queue" icon="≡" label={t("nav.queue")} /><NavItem route="logs" icon="⌁" label={t("nav.logs")} /><NavItem route="configuration" icon="◌" label={t("nav.configuration")} /><NavItem route="policy" icon="⚙" label={t("nav.policy")} /></nav><div className="sidebar-bottom"><div className="connection-card"><span className={`connection-dot${state.loaded && !state.error ? " online" : state.error ? " error" : ""}`} id="connection-dot"></span><div><strong id="connection-label">{state.loaded && !state.error ? t("connection.connected") : state.error ? t("connection.issue") : t("connection.connecting")}</strong><small id="connection-detail">{state.loaded && !state.error ? t("connection.synced") : state.error ? t("connection.retry") : t("connection.reading")}</small></div></div><div className="version-label">Stoker <span id="app-version">{state.config?.version || "—"}</span></div></div></aside>
    <main className="main-content" data-view={state.route}><header className="topbar"><div className="breadcrumbs"><span>{t("common.workspace")}</span><span className="crumb-separator">/</span><strong id="breadcrumb-current">{t(({ overview: "nav.overview", jobs: "nav.jobs", queue: "nav.queue", logs: "nav.logs", configuration: "nav.configuration", policy: "nav.policy" } as const)[state.route])}</strong></div><div className="topbar-actions"><LanguagePicker /><span className={`scheduler-pill ${state.status?.scheduler.running ? "running" : "stopped"}`} id="scheduler-pill"><span className="status-dot"></span><span id="scheduler-label">{state.status?.scheduler.running ? t("scheduler.running") : t("scheduler.stopped")}</span></span></div></header><section className="page" id="app" data-view={state.route} aria-live="polite">{!state.loaded ? (state.error ? <div className="page-heading"><div><div className="eyebrow">{t("workspace.unavailable")}</div><h1>{t("workspace.loadFailed")}</h1><p>{state.error}</p></div><button className="button primary" data-action="refresh" onClick={() => void actions.loadData()}>{t("common.retry")}</button></div> : <div className="page-loading"><span className="spinner"></span><span>{t("workspace.loading")}</span></div>) : <Page />}</section></main>
    <ToastRegion toasts={toasts} />
    <ModalDialog id="confirm-dialog" className="confirm-dialog" open={Boolean(confirmation)} onClose={() => actions.resolveConfirmation(false)}><div className="dialog-card confirm-card"><div className="dialog-kicker" id="confirm-kicker">{confirmation ? renderMessage(confirmation.kicker) : t("nav.configuration")}</div><h2 id="confirm-title">{confirmation ? renderMessage(confirmation.title) : t("common.confirmAction")}</h2><p id="confirm-message">{renderMessage(confirmation?.message || "")}</p><div className="dialog-actions"><button className="button secondary" id="confirm-cancel" type="button" onClick={() => actions.resolveConfirmation(false)}>{t("common.cancel")}</button><button className={`button ${confirmation?.destructive ? "danger" : "primary"}`} id="confirm-accept" type="button" onClick={() => actions.resolveConfirmation(true)}>{confirmation ? renderMessage(confirmation.acceptLabel) : t("common.confirm")}</button></div></div></ModalDialog>
    <ModalDialog id="job-dialog" className="job-dialog" open={jobFormOpen} onClose={actions.closeJobForm}><div className="job-dialog-shell" id="job-dialog-content"><JobForm /></div></ModalDialog>
    <ModalDialog id="job-detail-dialog" className="job-detail-dialog" open={jobDetailOpen} onClose={actions.closeJobDetail}><div className="job-detail-shell" id="job-detail-content">{detailEditing || state.selectedJob ? <JobDetail /> : <div className="page-loading"><span className="spinner"></span><span>{t("jobs.loading")}</span></div>}</div></ModalDialog>
  </div>;
}

function NavItem({ route, icon, label }: { route: "overview" | "jobs" | "queue" | "logs" | "configuration" | "policy"; icon: string; label: string }) {
  const { state } = useWorkspace();
  return <a className={`nav-item${state.route === route ? " active" : ""}`} href={`#${route}`} data-route={route}><span className="nav-icon">{icon}</span><span className="nav-label">{label}</span></a>;
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
