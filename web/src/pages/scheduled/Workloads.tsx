import { useLayoutEffect, useState } from "react";
import { EmptyState, LiveTime, PageHeading, Pagination, StateBadge } from "../../components";
import { pageInfo, useWorkspace } from "../../context";
import { clearLocalizedValidity, validateLocalizedForm } from "../../form-validation";
import { useI18n } from "../../i18n/context";
import { FlowDetail, ScheduleInputs, schedulePayload, type ScheduleInput } from "./FlowDetail";

export const SCHEDULED_FLOWS_PAGE_SIZE = 5;

export function Workloads() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const scheduled = state.scheduled;
  const [creating, setCreating] = useState(false);
  const [flowDetailScrollPending, setFlowDetailScrollPending] = useState(false);
  const [openedFlowScrollPending, setOpenedFlowScrollPending] = useState<string | null>(null);
  const [flowPage, setFlowPage] = useState(1);
  const flowsPage = pageInfo(scheduled.flows, flowPage, SCHEDULED_FLOWS_PAGE_SIZE);
  useLayoutEffect(() => {
    if (!creating) return;
    scrollWorkloadsToBottom();
  }, [creating]);
  useLayoutEffect(() => {
    if (!flowDetailScrollPending || !scheduled.selectedFlow || creating) return;
    const detail = document.getElementById("selected-flow-detail");
    if (!detail) return;
    detail.scrollIntoView({ block: "start" });
    setFlowDetailScrollPending(false);
  }, [flowDetailScrollPending, scheduled.selectedFlow, creating]);
  useLayoutEffect(() => {
    if (!openedFlowScrollPending || scheduled.selectedFlow?.flow_id !== openedFlowScrollPending) return;
    scrollWorkloadsToBottom();
    setOpenedFlowScrollPending(null);
  }, [openedFlowScrollPending, scheduled.selectedFlow]);
  return <>
    <PageHeading title={t("scheduled.workloads.title")} description={t("scheduled.workloads.description")} actions={<button className="button primary" data-action="new-flow" type="button" onClick={() => setCreating(true)}>{t("scheduled.flow.new")}</button>} />
    <div className="workload-tabs" role="tablist" aria-label={t("scheduled.workloads.tabs")}><button className={`workload-tab${scheduled.activeTab === "flows" ? " active" : ""}`} role="tab" aria-selected={scheduled.activeTab === "flows"} type="button" onClick={() => actions.selectScheduledTab("flows")}>{t("scheduled.flows")}</button><button className={`workload-tab${scheduled.activeTab === "jobs" ? " active" : ""}`} role="tab" aria-selected={scheduled.activeTab === "jobs"} type="button" onClick={() => actions.selectScheduledTab("jobs")}>{t("scheduled.standaloneJobs")}</button></div>
    {scheduled.activeTab === "flows" ? <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{t("scheduled.flows")}</h2><p>{t("scheduled.flowsDescription")}</p></div></div></div><div className="table-wrap"><table className="data-table workloads-table"><thead><tr><th>{t("scheduled.name")}</th><th>{t("scheduled.owner")}</th><th>{t("scheduled.schedule")}</th><th>{t("scheduled.tasks")}</th><th>{t("scheduled.status")}</th></tr></thead><tbody>{scheduled.flows.length ? flowsPage.items.map((flow) => <tr className="workload-row" tabIndex={0} key={flow.flow_id} onClick={() => void actions.openFlow(flow.flow_id).then(() => setOpenedFlowScrollPending(flow.flow_id))}><td><strong>{flow.name}</strong><small>{flow.flow_id}</small></td><td>{flow.owner}</td><td>{scheduleLabel(flow.schedule, t("scheduled.unscheduled"))}</td><td>{flow.tasks.length}</td><td><div className="flow-statuses"><StateBadge value={!flow.committed ? "DRAFT" : flow.enabled ? "RUNNING" : "CANCELLED"} />{flow.frozen && <StateBadge value="FROZEN" />}</div></td></tr>) : <tr><td colSpan={5}><EmptyState compact icon="◇" title={t("scheduled.noFlows")} /></td></tr>}</tbody></table></div><Pagination kind="flows" page={flowsPage} onPage={setFlowPage} /></section> : <ScheduledJobs />}
    {scheduled.selectedFlow && <FlowDetail flow={scheduled.selectedFlow} onClose={actions.closeScheduledFlow} />}
    {scheduled.selectedJob && <ScheduledJobDetail />}
    {creating && <NewFlowForm onClose={() => setCreating(false)} onCreated={() => { setCreating(false); setFlowDetailScrollPending(true); }} />}
  </>;
}

function scrollWorkloadsToBottom() {
  const scroller = document.querySelector<HTMLElement>(".main-content");
  if (scroller && getComputedStyle(scroller).overflowY !== "visible") scroller.scrollTo({ top: scroller.scrollHeight });
  else window.scrollTo({ top: document.documentElement.scrollHeight });
}

function NewFlowForm({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const [draft, setDraft] = useState({ flow_id: "", name: "", owner: "" });
  const defaultTimezone = state.settings?.effective_timezone.name || "UTC";
  const timezones = state.settings?.timezones || [defaultTimezone];
  const [schedule, setSchedule] = useState<ScheduleInput>({ kind: "once", date: "", time: "", timezone: defaultTimezone });
  const [scheduleError, setScheduleError] = useState<"scheduled.flow.chooseTimezone" | "scheduled.flow.invalidLocalTime" | null>(null);
  const update = (key: keyof typeof draft, next: string) => setDraft((current) => ({ ...current, [key]: next }));
  const save = async () => {
    const result = schedulePayload(schedule, timezones);
    if (!result.ok) {
      setScheduleError(result.reason === "timezone" ? "scheduled.flow.chooseTimezone" : "scheduled.flow.invalidLocalTime");
      return;
    }
    setScheduleError(null);
    if (await actions.createScheduledFlow({ ...draft, schedule: result.schedule })) onCreated();
  };
  return <section className="panel flow-detail"><form id="new-flow-form" className="new-flow-form" noValidate onInputCapture={(event) => clearLocalizedValidity(event.target)} onChangeCapture={(event) => clearLocalizedValidity(event.target)} onSubmit={(event) => { event.preventDefault(); if (validateLocalizedForm(event.currentTarget, t)) void save(); }}><div className="new-flow-form-heading"><div><h2>{t("scheduled.flow.new")}</h2><p>{t("scheduled.workloads.description")}</p></div></div><div className="new-flow-form-grid"><label><span>{t("scheduled.flow.taskId")}</span><input id="flow-id" className="text-input" required value={draft.flow_id} onChange={(event) => update("flow_id", event.target.value)} /></label><label><span>{t("scheduled.name")}</span><input id="flow-name" className="text-input" required value={draft.name} onChange={(event) => update("name", event.target.value)} /></label><label><span>{t("scheduled.owner")}</span><input id="flow-owner" className="text-input" required value={draft.owner} onChange={(event) => update("owner", event.target.value)} /></label></div><ScheduleInputs value={schedule} timezones={timezones} defaultTimezone={defaultTimezone} idPrefix="new-flow-schedule" onChange={(value) => { setSchedule(value); setScheduleError(null); }} />{scheduleError && <div className="form-feedback invalid schedule-error" aria-live="polite">{t(scheduleError)}</div>}<div className="new-flow-form-actions"><button className="button primary" type="submit">{t("scheduled.flow.create")}</button><button className="button secondary" type="button" onClick={onClose}>{t("common.cancel")}</button></div></form></section>;
}

function ScheduledJobs() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  return <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{t("scheduled.standaloneJobs")}</h2><p>{t("scheduled.jobsDescription")}</p></div></div></div><div className="table-wrap"><table className="data-table workloads-table"><thead><tr><th>{t("scheduled.name")}</th><th>{t("scheduled.owner")}</th><th>{t("scheduled.schedule")}</th><th>{t("scheduled.retry")}</th><th>{t("scheduled.status")}</th></tr></thead><tbody>{state.scheduled.jobs.length ? state.scheduled.jobs.map((item) => <tr className="workload-row" tabIndex={0} key={item.job.id} onClick={() => void actions.openScheduledJob(item.job.id)}><td><strong>{item.job.name}</strong><small>{item.job.id}</small></td><td>{item.job.user}</td><td>{scheduleLabel(item.definition.schedule, t("scheduled.unscheduled"))}</td><td>{item.definition.retry}</td><td><StateBadge value={item.job.state} /></td></tr>) : <tr><td colSpan={5}><EmptyState compact icon="◇" title={t("scheduled.noJobs")} /></td></tr>}</tbody></table></div></section>;
}

function ScheduledJobDetail() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const job = state.scheduled.selectedJob!;
  const run = () => void actions.scheduledJobMutation(job, "/runs", "POST", { replace_next: false });
  return <section className="panel scheduled-job-detail"><div className="panel-header"><div className="panel-title"><div><h2>{job.job.name}</h2><p>{job.job.cwd}</p></div></div><div className="page-actions">{job.job.state === "DRAFT" ? <button className="button primary" type="button" onClick={() => void actions.scheduledJobMutation(job, "/commit", "POST")}>{t("scheduled.flow.commit")}</button> : <><button className="button secondary" type="button" onClick={run}>{t("scheduled.flow.runNow")}</button><button className="button secondary" type="button" onClick={() => void actions.scheduledJobMutation(job, "/freeze", "POST")}>{t("scheduled.flow.freeze")}</button><button className="button danger" type="button" onClick={() => void actions.scheduledJobMutation(job, "/disable", "POST")}>{t("scheduled.flow.disable")}</button></>}</div></div><div className="detail-grid"><div className="detail-item"><label>{t("scheduled.command")}</label><code>{job.job.command_line || job.job.command?.join(" ") || "—"}</code></div><div className="detail-item"><label>{t("scheduled.schedule")}</label><span>{scheduleLabel(job.definition.schedule, t("scheduled.unscheduled"))}</span></div><div className="detail-item"><label>{t("scheduled.created")}</label><span><LiveTime value={job.job.created_at} timezone={state.timezone?.name} /></span></div><div className="detail-item"><label>{t("scheduled.retry")}</label><span>{job.definition.retry}</span></div></div></section>;
}

export function scheduleLabel(schedule: { kind: string; at?: string; time?: string; timezone?: string; every?: string } | null, empty: string) {
  if (!schedule) return empty;
  if (schedule.kind === "daily") return `${schedule.time} · ${schedule.timezone}`;
  return schedule.kind === "periodic" ? schedule.every || empty : schedule.at || empty;
}
