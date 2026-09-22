import { useEffect, useRef, useState } from "react";
import { EmptyState, StateBadge, TimePicker, TimezonePicker } from "../../components";
import { useWorkspace } from "../../context";
import { useI18n } from "../../i18n/context";
import { scheduledSelectors } from "../../scheduled/state";
import { instantToLocalDateTime, localDateTimeToRfc3339 } from "../../scheduled/timezone";
import type { ScheduledFlow, ScheduledSchedule, ScheduledTask } from "../../types";
import { scheduleLabel } from "./Workloads";

const EMPTY_TASK = { task_id: "", name: "", cwd: "", command: "", retry: 0, dependencies: "", depend_mode: "all" };
type ScheduleInput =
  | { kind: "once"; date: string; time: string; timezone: string }
  | { kind: "daily"; time: string; timezone: string }
  | { kind: "periodic"; every: string; first_at: string | null };

export function FlowDetail({ flow }: { flow: ScheduledFlow }) {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const effectiveTimezone = state.settings?.effective_timezone.name || "UTC";
  const timezones = state.settings?.timezones || [effectiveTimezone];
  const [editingSchedule, setEditingSchedule] = useState(false);
  const [schedule, setSchedule] = useState<ScheduleInput>(() => toScheduleInput(flow, effectiveTimezone));
  const [scheduleError, setScheduleError] = useState("");
  const previousFlowId = useRef(flow.flow_id);
  const serverScheduleKey = JSON.stringify(flow.schedule);
  useEffect(() => {
    const sameFlow = previousFlowId.current === flow.flow_id;
    setSchedule((current) => toScheduleInput(flow, sameFlow && current.kind === "once" ? current.timezone : effectiveTimezone));
    setScheduleError("");
    previousFlowId.current = flow.flow_id;
  }, [flow.flow_id, serverScheduleKey, effectiveTimezone]);
  const sync = scheduledSelectors.isSyncSource(state.scheduled);
  const actionState = scheduledSelectors.flowActionState(flow);
  const mutate = (path: string, method: string, body?: Record<string, unknown>) => void actions.scheduledFlowMutation(flow, path, method, body);
  const updateSchedule = (value: ScheduleInput) => { setSchedule(value); setScheduleError(""); };
  const saveSchedule = () => {
    const result = schedulePayload(schedule, timezones);
    if (!result.ok) {
      setScheduleError(t(result.reason === "timezone" ? "scheduled.flow.chooseTimezone" : "scheduled.flow.invalidLocalTime"));
      return;
    }
    setScheduleError("");
    mutate("/schedule", "PUT", { schedule: result.schedule });
  };
  return <section className="panel flow-detail"><div className="panel-header"><div className="panel-title"><div><div className="section-kicker">{t("scheduled.flow.detail")}</div><h2>{flow.name}</h2><p>{flow.flow_id} · {flow.owner}</p></div></div><div className="page-actions"><FlowActions flow={flow} sync={sync} mutate={mutate} /></div></div>
    {sync && <div className="flow-notice">{t("scheduled.flow.syncReadOnly")}</div>}
    {state.scheduled.revisionConflict && <div className="flow-notice conflict"><span>{t("scheduled.flow.revisionConflict")}</span><button className="button small secondary" type="button" onClick={() => void actions.openFlow(flow.flow_id)}>{t("scheduled.flow.reload")}</button></div>}
    <div className="flow-meta"><div><span>{t("scheduled.schedule")}</span><strong>{scheduleLabel(flow.schedule, t("scheduled.unscheduled"))}</strong></div><div><span>{t("scheduled.tasks")}</span><strong>{flow.tasks.length}</strong></div><div><span>{t("scheduled.status")}</span><StateBadge value={!flow.committed ? "DRAFT" : flow.enabled ? "RUNNING" : "CANCELLED"} /></div></div>
    {actionState === "frozen" && !sync && <div className="flow-schedule-editor"><button className="button small secondary" type="button" onClick={() => setEditingSchedule((value) => !value)}>{t("scheduled.flow.editSchedule")}</button>{editingSchedule && <form onSubmit={(event) => { event.preventDefault(); saveSchedule(); }}><ScheduleInputs value={schedule} timezones={timezones} defaultTimezone={effectiveTimezone} onChange={updateSchedule} /><button className="button small primary" type="submit">{t("common.save")}</button>{scheduleError && <div className="form-feedback invalid schedule-error" aria-live="polite">{scheduleError}</div>}</form>}</div>}
    <section className="flow-section"><div className="section-heading-row"><div><div className="section-kicker">{t("scheduled.flow.graph")}</div><h3>{t("scheduled.flow.tasks")}</h3></div></div><TaskGraph tasks={flow.tasks} />{(actionState === "draft" || actionState === "frozen") && !sync && <TaskEditor flow={flow} />}{!flow.tasks.length && <EmptyState compact icon="◇" title={t("scheduled.flow.noTasks")} />}</section>
  </section>;
}

function FlowActions({ flow, sync, mutate }: { flow: ScheduledFlow; sync: boolean; mutate: (path: string, method: string, body?: Record<string, unknown>) => void }) {
  const { t } = useI18n();
  const { actions } = useWorkspace();
  if (sync) return <><button className="button secondary" type="button" onClick={() => mutate("/runs", "POST", { replace_next: false })}>{t("scheduled.flow.runNow")}</button><a className="button secondary" href="#logs">{t("scheduled.flow.logs")}</a></>;
  if (!flow.committed) return <><button className="button primary" type="button" onClick={() => document.getElementById("flow-task-id")?.focus()}>{t("scheduled.flow.addTask")}</button><button className="button primary" type="button" disabled={!flow.tasks.length} onClick={() => mutate("/commit", "POST")}>{t("scheduled.flow.commit")}</button><button className="button danger" type="button" onClick={() => void actions.deleteScheduledDraftFlow(flow).then((deleted) => { if (deleted) actions.navigate("workloads"); })}>{t("scheduled.flow.delete")}</button></>;
  if (!flow.frozen) return <><button className="button secondary" type="button" onClick={() => mutate("/runs", "POST", { replace_next: false })}>{t("scheduled.flow.runNow")}</button><button className="button secondary" type="button" onClick={() => mutate("/freeze", "POST")}>{t("scheduled.flow.freeze")}</button><button className="button danger" type="button" onClick={() => mutate("/disable", "POST")}>{t("scheduled.flow.disable")}</button></>;
  if (!flow.has_draft) return <button className="button primary" type="button" onClick={() => mutate("/unfreeze", "POST")}>{t("scheduled.flow.unfreeze")}</button>;
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

function toScheduleInput(flow: ScheduledFlow, timezone: string): ScheduleInput {
  const schedule = flow.schedule;
  if (!schedule || schedule.kind === "once") {
    const local = schedule?.at ? instantToLocalDateTime(schedule.at, timezone) : null;
    return { kind: "once", date: local?.date || "", time: local?.time || "", timezone };
  }
  if (schedule.kind === "daily") return { kind: "daily", time: schedule.time, timezone: schedule.timezone };
  return { kind: "periodic", every: schedule.every, first_at: schedule.first_at || null };
}

function schedulePayload(value: ScheduleInput, timezones: string[]): { ok: true; schedule: ScheduledSchedule } | { ok: false; reason: "timezone" | "local-time" } {
  if (value.kind === "periodic") return { ok: true, schedule: value };
  if (!timezones.includes(value.timezone.trim())) return { ok: false, reason: "timezone" };
  if (value.kind === "daily") return { ok: true, schedule: { ...value, timezone: value.timezone.trim() } };
  const converted = localDateTimeToRfc3339(value.date, value.time, value.timezone.trim());
  return converted.ok
    ? { ok: true, schedule: { kind: "once", at: converted.value } }
    : { ok: false, reason: "local-time" };
}

function ScheduleInputs({ value, timezones, defaultTimezone, onChange }: { value: ScheduleInput; timezones: string[]; defaultTimezone: string; onChange: (value: ScheduleInput) => void }) {
  const { t } = useI18n();
  const timezoneField = (timezone: string, update: (timezone: string) => void) => <label className="schedule-field schedule-timezone-field"><span>{t("scheduled.flow.timezone")}</span><TimezonePicker id="schedule-timezone" suggestionsId="schedule-timezone-suggestions" value={timezone} timezones={timezones} placeholder={t("config.timezonePlaceholder")} required onChange={update} /></label>;
  const timeField = (time: string, update: (time: string) => void) => <label className="schedule-field schedule-time-field"><span>{t("scheduled.flow.time")}</span><TimePicker id="schedule-time" value={time} hourLabel={t("scheduled.flow.hour")} minuteLabel={t("scheduled.flow.minute")} required onChange={update} /></label>;
  return <div className="schedule-inputs"><label className="schedule-field schedule-kind-field"><span>{t("scheduled.flow.scheduleType")}</span><select className="select-input" value={value.kind} onChange={(event) => onChange(event.target.value === "daily" ? { kind: "daily", time: "00:00", timezone: defaultTimezone } : event.target.value === "periodic" ? { kind: "periodic", every: "1h", first_at: null } : { kind: "once", date: "", time: "", timezone: defaultTimezone })}><option value="once">{t("scheduled.flow.once")}</option><option value="daily">{t("scheduled.flow.daily")}</option><option value="periodic">{t("scheduled.flow.periodic")}</option></select></label>{value.kind === "once" ? <><label className="schedule-field"><span>{t("scheduled.flow.date")}</span><input className="text-input" type="date" required value={value.date} onChange={(event) => onChange({ ...value, date: event.target.value })} /></label>{timeField(value.time, (time) => onChange({ ...value, time }))}{timezoneField(value.timezone, (timezone) => onChange({ ...value, timezone }))}</> : value.kind === "daily" ? <>{timeField(value.time, (time) => onChange({ ...value, time }))}{timezoneField(value.timezone, (timezone) => onChange({ ...value, timezone }))}</> : <label className="schedule-field"><span>{t("scheduled.flow.interval")}</span><input className="text-input" required value={value.every} onChange={(event) => onChange({ ...value, every: event.target.value })} /></label>}</div>;
}
