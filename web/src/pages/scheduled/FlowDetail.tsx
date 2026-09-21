import { useState } from "react";
import { EmptyState, StateBadge } from "../../components";
import { useWorkspace } from "../../context";
import { useI18n } from "../../i18n/context";
import { scheduledSelectors } from "../../scheduled/state";
import type { ScheduledFlow, ScheduledTask } from "../../types";
import { scheduleLabel } from "./Workloads";

const EMPTY_TASK = { task_id: "", name: "", cwd: "", command: "", retry: 0, dependencies: "", depend_mode: "all" };

export function FlowDetail({ flow }: { flow: ScheduledFlow }) {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const [editingSchedule, setEditingSchedule] = useState(false);
  const [schedule, setSchedule] = useState(() => toScheduleInput(flow));
  const sync = scheduledSelectors.isSyncSource(state.scheduled);
  const actionState = scheduledSelectors.flowActionState(flow);
  const mutate = (path: string, method: string, body?: Record<string, unknown>) => void actions.scheduledFlowMutation(flow, path, method, body);
  return <section className="panel flow-detail"><div className="panel-header"><div className="panel-title"><div><div className="section-kicker">{t("scheduled.flow.detail")}</div><h2>{flow.name}</h2><p>{flow.flow_id} · {flow.owner}</p></div></div><div className="page-actions"><FlowActions flow={flow} sync={sync} mutate={mutate} /></div></div>
    {sync && <div className="flow-notice">{t("scheduled.flow.syncReadOnly")}</div>}
    {state.scheduled.revisionConflict && <div className="flow-notice conflict"><span>{t("scheduled.flow.revisionConflict")}</span><button className="button small secondary" type="button" onClick={() => void actions.openFlow(flow.flow_id)}>{t("scheduled.flow.reload")}</button></div>}
    <div className="flow-meta"><div><span>{t("scheduled.schedule")}</span><strong>{scheduleLabel(flow.schedule, t("scheduled.unscheduled"))}</strong></div><div><span>{t("scheduled.tasks")}</span><strong>{flow.tasks.length}</strong></div><div><span>{t("scheduled.status")}</span><StateBadge value={!flow.committed ? "DRAFT" : flow.enabled ? "RUNNING" : "CANCELLED"} /></div></div>
    {actionState === "frozen" && !sync && <div className="flow-schedule-editor"><button className="button small secondary" type="button" onClick={() => setEditingSchedule((value) => !value)}>{t("scheduled.flow.editSchedule")}</button>{editingSchedule && <form onSubmit={(event) => { event.preventDefault(); mutate("/schedule", "PUT", schedule); }}><ScheduleInputs value={schedule} onChange={setSchedule} /><button className="button small primary" type="submit">{t("common.save")}</button></form>}</div>}
    <section className="flow-section"><div className="section-heading-row"><div><div className="section-kicker">{t("scheduled.flow.graph")}</div><h3>{t("scheduled.flow.tasks")}</h3></div></div><TaskGraph tasks={flow.tasks} />{(actionState === "draft" || actionState === "frozen") && !sync && <TaskEditor flow={flow} />}{!flow.tasks.length && <EmptyState compact icon="◇" title={t("scheduled.flow.noTasks")} />}</section>
  </section>;
}

function FlowActions({ flow, sync, mutate }: { flow: ScheduledFlow; sync: boolean; mutate: (path: string, method: string, body?: Record<string, unknown>) => void }) {
  const { t } = useI18n();
  const { actions } = useWorkspace();
  if (sync) return <><button className="button secondary" type="button" onClick={() => mutate("/runs", "POST", { replace_next: false })}>{t("scheduled.flow.runNow")}</button><a className="button secondary" href="#logs">{t("scheduled.flow.logs")}</a><a className="button secondary" href="#sources">{t("scheduled.flow.export")}</a><a className="button secondary" href="#sources">{t("scheduled.flow.snapshot")}</a></>;
  if (!flow.committed) return <><button className="button primary" type="button" onClick={() => document.getElementById("flow-task-id")?.focus()}>{t("scheduled.flow.addTask")}</button><button className="button primary" type="button" disabled={!flow.tasks.length} onClick={() => mutate("/commit", "POST")}>{t("scheduled.flow.commit")}</button><button className="button danger" type="button" onClick={() => void actions.deleteScheduledDraftFlow(flow).then((deleted) => { if (deleted) actions.navigate("workloads"); })}>{t("scheduled.flow.delete")}</button></>;
  if (!flow.frozen) return <><button className="button secondary" type="button" onClick={() => mutate("/runs", "POST", { replace_next: false })}>{t("scheduled.flow.runNow")}</button><button className="button secondary" type="button" onClick={() => mutate("/freeze", "POST")}>{t("scheduled.flow.freeze")}</button><button className="button danger" type="button" onClick={() => mutate("/disable", "POST")}>{t("scheduled.flow.disable")}</button></>;
  return <><button className="button primary" type="button" onClick={() => mutate("/apply", "POST")}>{t("scheduled.flow.apply")}</button><button className="button secondary" type="button" onClick={() => mutate("/discard", "POST")}>{t("scheduled.flow.discard")}</button></>;
}

function TaskGraph({ tasks }: { tasks: ScheduledTask[] }) {
  const { t } = useI18n();
  const { actions } = useWorkspace();
  const flow = useWorkspace().state.scheduled.selectedFlow;
  return <div className="flow-graph" aria-label={t("scheduled.flow.graph")}><div className="flow-links" aria-hidden="true">{tasks.flatMap((task) => task.dependencies.map((dependency) => <span key={`${task.task_id}:${dependency.task_id}`}>{dependency.task_id} → {task.task_id}</span>))}</div>{tasks.map((task) => <article className="task-card" key={task.task_id}><div className="task-card-heading"><strong>{task.name}</strong><code>{task.task_id}</code></div><code>{task.command}</code><small>{task.dependencies.length ? `${t("scheduled.flow.dependsOn")} ${task.dependencies.map((dependency) => dependency.task_id).join(", ")}` : t("scheduled.flow.rootTask")}</small>{flow?.frozen && <div className="task-card-actions"><button className="button small secondary" type="button" onClick={() => { const command = window.prompt(t("scheduled.flow.editTask"), task.command); if (command !== null) void actions.scheduledFlowMutation(flow, `/tasks/${encodeURIComponent(task.task_id)}`, "PATCH", { command }); }}>{t("common.edit")}</button><button className="button small danger" type="button" onClick={() => void actions.scheduledFlowMutation(flow, `/tasks/${encodeURIComponent(task.task_id)}`, "DELETE")}>{t("scheduled.flow.removeTask")}</button></div>}</article>)}</div>;
}

function TaskEditor({ flow }: { flow: ScheduledFlow }) {
  const { t } = useI18n();
  const { actions } = useWorkspace();
  const [draft, setDraft] = useState(EMPTY_TASK);
  const update = (key: keyof typeof draft, value: string | number) => setDraft((current) => ({ ...current, [key]: value }));
  return <form className="task-editor" onSubmit={(event) => { event.preventDefault(); const dependencies = draft.dependencies.split(",").map((value) => value.trim()).filter(Boolean).map((task_id) => ({ task_id, state: "succeeded" })); void actions.scheduledFlowMutation(flow, "/tasks", "POST", { task_id: draft.task_id, name: draft.name, cwd: draft.cwd, command: draft.command, retry: Number(draft.retry), dependencies, depend_mode: draft.depend_mode }).then((saved) => { if (saved) setDraft(EMPTY_TASK); }); }}><h3>{t("scheduled.flow.addTask")}</h3><div className="task-editor-grid"><label>{t("scheduled.flow.taskId")}<input id="flow-task-id" className="text-input" required value={draft.task_id} onChange={(event) => update("task_id", event.target.value)} /></label><label>{t("scheduled.name")}<input className="text-input" required value={draft.name} onChange={(event) => update("name", event.target.value)} /></label><label>{t("scheduled.command")}<input className="text-input" required value={draft.command} onChange={(event) => update("command", event.target.value)} /></label><label>{t("scheduled.directory")}<input className="text-input" required value={draft.cwd} onChange={(event) => update("cwd", event.target.value)} /></label><label>{t("scheduled.retry")}<input className="text-input" type="number" min="0" value={draft.retry} onChange={(event) => update("retry", event.target.value)} /></label><label>{t("scheduled.flow.dependencies")}<input className="text-input" value={draft.dependencies} onChange={(event) => update("dependencies", event.target.value)} /></label></div><button className="button small primary" type="submit">{t("scheduled.flow.addTask")}</button></form>;
}

function toScheduleInput(flow: ScheduledFlow) {
  const schedule = flow.schedule;
  if (!schedule || schedule.kind === "once") return { kind: "once", at: schedule?.at || "" } as const;
  if (schedule.kind === "daily") return { kind: "daily", time: schedule.time, timezone: schedule.timezone } as const;
  return { kind: "periodic", every: schedule.every, first_at: schedule.first_at || null } as const;
}

function ScheduleInputs({ value, onChange }: { value: ReturnType<typeof toScheduleInput>; onChange: (value: ReturnType<typeof toScheduleInput>) => void }) {
  const { t } = useI18n();
  return <div className="schedule-inputs"><select className="select-input" value={value.kind} onChange={(event) => onChange(event.target.value === "daily" ? { kind: "daily", time: "00:00", timezone: "UTC" } : event.target.value === "periodic" ? { kind: "periodic", every: "1h", first_at: null } : { kind: "once", at: "" })}><option value="once">{t("scheduled.flow.once")}</option><option value="daily">{t("scheduled.flow.daily")}</option><option value="periodic">{t("scheduled.flow.periodic")}</option></select>{value.kind === "once" ? <input className="text-input" value={value.at} onChange={(event) => onChange({ ...value, at: event.target.value })} /> : value.kind === "daily" ? <><input className="text-input" value={value.time} onChange={(event) => onChange({ ...value, time: event.target.value })} /><input className="text-input" value={value.timezone} onChange={(event) => onChange({ ...value, timezone: event.target.value })} /></> : <input className="text-input" value={value.every} onChange={(event) => onChange({ ...value, every: event.target.value })} />}</div>;
}
