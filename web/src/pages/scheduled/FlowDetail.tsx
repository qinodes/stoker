import { useEffect, useRef, useState } from "react";
import { EmptyState, StateBadge, TimePicker, TimezonePicker } from "../../components";
import { useWorkspace } from "../../context";
import { clearLocalizedValidity, validateLocalizedForm } from "../../form-validation";
import { useI18n } from "../../i18n/context";
import { scheduledSelectors } from "../../scheduled/state";
import { instantToLocalDateTime, localDateTimeToRfc3339 } from "../../scheduled/timezone";
import type { ScheduledFlow, ScheduledSchedule, ScheduledTask } from "../../types";
import { scheduleLabel } from "./Workloads";

const EMPTY_TASK = { task_id: "", name: "", cwd: "", command: "", retry: 0, dependencies: "", depend_mode: "all" };
export type ScheduleInput =
  | { kind: "once"; date: string; time: string; timezone: string }
  | { kind: "daily"; time: string; timezone: string }
  | { kind: "periodic"; every: string; first_at: string | null };

export function FlowDetail({ flow, onClose }: { flow: ScheduledFlow; onClose: () => void }) {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const effectiveTimezone = state.settings?.effective_timezone.name || "UTC";
  const timezones = state.settings?.timezones || [effectiveTimezone];
  const [editingSchedule, setEditingSchedule] = useState(false);
  const [schedule, setSchedule] = useState<ScheduleInput>(() => toScheduleInput(flow, effectiveTimezone));
  const [scheduleError, setScheduleError] = useState<"scheduled.flow.chooseTimezone" | "scheduled.flow.invalidLocalTime" | null>(null);
  const previousFlowId = useRef(flow.flow_id);
  const serverScheduleKey = JSON.stringify(flow.schedule);
  useEffect(() => {
    const sameFlow = previousFlowId.current === flow.flow_id;
    setSchedule((current) => toScheduleInput(flow, sameFlow && current.kind === "once" ? current.timezone : effectiveTimezone));
    setScheduleError(null);
    previousFlowId.current = flow.flow_id;
  }, [flow.flow_id, serverScheduleKey, effectiveTimezone]);
  const sync = scheduledSelectors.isSyncSource(state.scheduled);
  const actionState = scheduledSelectors.flowActionState(flow);
  const mutate = (path: string, method: string, body?: Record<string, unknown>) => void actions.scheduledFlowMutation(flow, path, method, body);
  const updateSchedule = (value: ScheduleInput) => { setSchedule(value); setScheduleError(null); };
  const saveSchedule = () => {
    const result = schedulePayload(schedule, timezones);
    if (!result.ok) {
      setScheduleError(result.reason === "timezone" ? "scheduled.flow.chooseTimezone" : "scheduled.flow.invalidLocalTime");
      return;
    }
    setScheduleError(null);
    mutate("/schedule", "PUT", { schedule: result.schedule });
  };
  return <section className="panel flow-detail" id="selected-flow-detail"><div className="panel-header"><div className="panel-title"><div><div className="section-kicker">{t("scheduled.flow.detail")}</div><h2>{flow.name}</h2><p>{flow.flow_id} · {flow.owner}</p></div></div><div className="page-actions"><FlowActions flow={flow} sync={sync} mutate={mutate} /></div></div>
    {sync && <div className="flow-notice">{t("scheduled.flow.syncReadOnly")}</div>}
    {state.scheduled.revisionConflict && <div className="flow-notice conflict"><span>{t("scheduled.flow.revisionConflict")}</span><button className="button small secondary" type="button" onClick={() => void actions.openFlow(flow.flow_id)}>{t("scheduled.flow.reload")}</button></div>}
    <div className="flow-meta"><div><span className="flow-meta-label">{t("scheduled.schedule")}</span><strong className="flow-meta-value">{scheduleLabel(flow.schedule, t("scheduled.unscheduled"))}</strong></div><div><span className="flow-meta-label">{t("scheduled.tasks")}</span><strong className="flow-meta-value">{flow.tasks.length}</strong></div><div><span className="flow-meta-label">{t("scheduled.status")}</span><StateBadge value={!flow.committed ? "DRAFT" : flow.enabled ? "RUNNING" : "CANCELLED"} /></div></div>
    {actionState === "frozen" && !sync && <div className="flow-schedule-editor"><button className="button small secondary" type="button" onClick={() => setEditingSchedule((value) => !value)}>{t("scheduled.flow.editSchedule")}</button>{editingSchedule && <form noValidate onInputCapture={(event) => clearLocalizedValidity(event.target)} onChangeCapture={(event) => clearLocalizedValidity(event.target)} onSubmit={(event) => { event.preventDefault(); if (validateLocalizedForm(event.currentTarget, t)) saveSchedule(); }}><ScheduleInputs value={schedule} timezones={timezones} defaultTimezone={effectiveTimezone} onChange={updateSchedule} /><button className="button small primary" type="submit">{t("common.save")}</button>{scheduleError && <div className="form-feedback invalid schedule-error" aria-live="polite">{t(scheduleError)}</div>}</form>}</div>}
    <section className="flow-section"><div className="section-heading-row"><div><div className="section-kicker">{t("scheduled.flow.graph")}</div><h3>{t("scheduled.flow.tasks")}</h3></div></div><TaskGraph tasks={flow.tasks} />{(actionState === "draft" || actionState === "frozen") && !sync && <TaskEditor flow={flow} />}{!flow.tasks.length && <EmptyState compact icon="◇" title={t("scheduled.flow.noTasks")} />}</section><div className="flow-detail-close"><button className="button small secondary" type="button" onClick={onClose}>{t("scheduled.flow.closeDetail")}</button></div>
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
  const { state, actions } = useWorkspace();
  const flow = state.scheduled.selectedFlow;
  const [editingTask, setEditingTask] = useState<ScheduledTask | null>(null);
  return <><div className="flow-graph" aria-label={t("scheduled.flow.graph")}><div className="flow-links" aria-hidden="true">{tasks.flatMap((task) => task.dependencies.map((dependency) => <span key={`${task.task_id}:${dependency.task_id}`}>{dependency.task_id} → {task.task_id}</span>))}</div>{tasks.map((task) => <article className="task-card" key={task.task_id}><div className="task-card-heading"><strong>{task.name}</strong><code>{task.task_id}</code></div><code>{task.command}</code><small>{task.dependencies.length ? `${t("scheduled.flow.dependsOn")} ${task.dependencies.map((dependency) => dependency.task_id).join(", ")}` : t("scheduled.flow.rootTask")}</small>{flow?.frozen && <div className="task-card-actions"><button className="button small secondary" type="button" onClick={() => setEditingTask(task)}>{t("common.edit")}</button><button className="button small danger" type="button" onClick={() => void actions.scheduledFlowMutation(flow, `/tasks/${encodeURIComponent(task.task_id)}`, "DELETE")}>{t("scheduled.flow.removeTask")}</button></div>}</article>)}</div>{flow && editingTask && <TaskEditDialog flow={flow} task={editingTask} onClose={() => setEditingTask(null)} />}</>;
}

function TaskEditDialog({ flow, task, onClose }: { flow: ScheduledFlow; task: ScheduledTask; onClose: () => void }) {
  const { t } = useI18n();
  const { actions } = useWorkspace();
  const dialog = useRef<HTMLDialogElement>(null);
  const [command, setCommand] = useState(task.command);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (!dialog.current?.open) dialog.current?.showModal();
  }, []);
  const save = async () => {
    if (!command.trim() || saving) return;
    setSaving(true);
    const saved = await actions.scheduledFlowMutation(flow, `/tasks/${encodeURIComponent(task.task_id)}`, "PATCH", { command });
    setSaving(false);
    if (saved) onClose();
  };
  return <dialog ref={dialog} className="confirm-dialog task-edit-dialog" aria-labelledby="task-edit-title" onCancel={(event) => { event.preventDefault(); onClose(); }} onClick={(event) => { if (event.target === event.currentTarget) onClose(); }}><form className="dialog-card task-edit-card" onSubmit={(event) => { event.preventDefault(); void save(); }}><h2 id="task-edit-title">{t("scheduled.flow.editTask")}</h2><p>{task.name}</p><label htmlFor="task-edit-command">{t("scheduled.command")}</label><textarea id="task-edit-command" autoFocus required value={command} onChange={(event) => setCommand(event.target.value)} /><div className="dialog-actions"><button className="button secondary" type="button" onClick={onClose}>{t("common.cancel")}</button><button className="button primary" type="submit" disabled={saving || !command.trim()}>{t("common.save")}</button></div></form></dialog>;
}

function TaskEditor({ flow }: { flow: ScheduledFlow }) {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const [draft, setDraft] = useState(EMPTY_TASK);
  const update = (key: keyof typeof draft, value: string | number) => setDraft((current) => ({ ...current, [key]: value }));
  return <form className="task-editor" noValidate onInputCapture={(event) => clearLocalizedValidity(event.target)} onChangeCapture={(event) => clearLocalizedValidity(event.target)} onSubmit={(event) => { event.preventDefault(); if (!validateLocalizedForm(event.currentTarget, t)) return; const dependencies = draft.dependencies.split(",").map((value) => value.trim()).filter(Boolean).map((task_id) => ({ task_id, state: "succeeded" })); void actions.scheduledFlowMutation(flow, "/tasks", "POST", { task_id: draft.task_id, name: draft.name, cwd: draft.cwd, command: draft.command, retry: Number(draft.retry), dependencies, depend_mode: draft.depend_mode }).then((saved) => { if (saved) setDraft(EMPTY_TASK); }); }}><h3>{t("scheduled.flow.addTask")}</h3><div className="task-editor-grid"><label>{t("scheduled.flow.taskId")}<input id="flow-task-id" className="text-input" required value={draft.task_id} onChange={(event) => update("task_id", event.target.value)} /></label><label>{t("scheduled.name")}<input className="text-input" required value={draft.name} onChange={(event) => update("name", event.target.value)} /></label><label>{t("scheduled.command")}<input className="text-input" required value={draft.command} onChange={(event) => update("command", event.target.value)} /></label><label><span>{t("scheduled.directory")}</span><div className="path-input-row"><input id="flow-task-cwd" className="text-input mono-input" required value={draft.cwd} onChange={(event) => update("cwd", event.target.value)} /><button className="button secondary" data-action="browse-scheduled-directory" type="button" onClick={() => void actions.openDirectoryBrowser(draft.cwd || state.filesystem.roots?.default_path || "", (path) => update("cwd", path))}>{t("jobs.browse")}</button></div></label><label>{t("scheduled.retry")}<input className="text-input" type="number" min="0" value={draft.retry} onChange={(event) => update("retry", event.target.value)} /></label><label>{t("scheduled.flow.dependencies")}<input className="text-input" value={draft.dependencies} onChange={(event) => update("dependencies", event.target.value)} /></label></div><button className="button small primary" type="submit">{t("scheduled.flow.addTask")}</button></form>;
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

export function schedulePayload(value: ScheduleInput, timezones: string[]): { ok: true; schedule: ScheduledSchedule } | { ok: false; reason: "timezone" | "local-time" } {
  if (value.kind === "periodic") return { ok: true, schedule: value };
  if (!timezones.includes(value.timezone.trim())) return { ok: false, reason: "timezone" };
  if (value.kind === "daily") return { ok: true, schedule: { ...value, timezone: value.timezone.trim() } };
  const converted = localDateTimeToRfc3339(value.date, value.time, value.timezone.trim());
  return converted.ok
    ? { ok: true, schedule: { kind: "once", at: converted.value } }
    : { ok: false, reason: "local-time" };
}

type CalendarDate = { year: number; month: number; day: number };
type CalendarMonth = Omit<CalendarDate, "day">;

function validCalendarDate(year: number, month: number, day: number): boolean {
  if (year < 1000 || month < 1 || month > 12 || day < 1 || day > 31) return false;
  const date = new Date(Date.UTC(year, month - 1, day));
  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day;
}

function toIsoDate({ year, month, day }: CalendarDate): string {
  return `${String(year).padStart(4, "0")}-${String(month + 1).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

function parseScheduleDate(value: string, locale: string): string | null {
  const normalized = value.trim();
  const iso = /^(\d{4})-(\d{2})-(\d{2})$/.exec(normalized);
  if (iso) {
    const [, year, month, day] = iso.map(Number);
    return validCalendarDate(year, month, day) ? normalized : null;
  }
  const localized = locale === "en" ? /^(\d{1,2})[/-](\d{1,2})[/-](\d{4})$/ : /^(\d{4})[/-](\d{1,2})[/-](\d{1,2})$/;
  const match = localized.exec(normalized);
  if (!match) return null;
  const parts = match.slice(1).map(Number);
  const [year, month, day] = locale === "en" ? [parts[2], parts[0], parts[1]] : parts;
  return validCalendarDate(year, month, day) ? toIsoDate({ year, month: month - 1, day }) : null;
}

function formatScheduleDate(value: string, locale: string): string {
  const iso = parseScheduleDate(value, "en") || parseScheduleDate(value, "zh-TW");
  if (!iso) return value;
  const [, year, month, day] = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso) || [];
  return locale === "en" ? `${month}/${day}/${year}` : `${year}/${month}/${day}`;
}

function calendarMonth(value?: string): CalendarMonth {
  const match = value && /^(\d{4})-(\d{2})-\d{2}$/.exec(value);
  if (match) return { year: Number(match[1]), month: Number(match[2]) - 1 };
  const now = new Date();
  return { year: now.getFullYear(), month: now.getMonth() };
}

function todayIso(): string {
  const now = new Date();
  return toIsoDate({ year: now.getFullYear(), month: now.getMonth(), day: now.getDate() });
}

function shiftCalendarMonth(value: CalendarMonth, amount: number): CalendarMonth {
  const date = new Date(Date.UTC(value.year, value.month + amount, 1));
  return { year: date.getUTCFullYear(), month: date.getUTCMonth() };
}

function calendarDays(value: CalendarMonth): Array<{ iso: string; day: number; outside: boolean }> {
  const first = new Date(Date.UTC(value.year, value.month, 1));
  const firstWeekday = first.getUTCDay();
  const daysInMonth = new Date(Date.UTC(value.year, value.month + 1, 0)).getUTCDate();
  const count = Math.ceil((firstWeekday + daysInMonth) / 7) * 7;
  return Array.from({ length: count }, (_, index) => {
    const date = new Date(Date.UTC(value.year, value.month, index - firstWeekday + 1));
    return { iso: toIsoDate({ year: date.getUTCFullYear(), month: date.getUTCMonth(), day: date.getUTCDate() }), day: date.getUTCDate(), outside: date.getUTCMonth() !== value.month };
  });
}

function calendarLocale(locale: string): string {
  return locale === "en" ? "en-US" : locale === "ja" ? "ja-JP" : "zh-TW";
}

function formatCalendarDate(value: string, locale: string, options: Intl.DateTimeFormatOptions): string {
  const [year, month, day] = value.split("-").map(Number);
  return new Intl.DateTimeFormat(calendarLocale(locale), { ...options, timeZone: "UTC" }).format(new Date(Date.UTC(year, month - 1, day)));
}

function LocalizedDateInput({ id, value, required, onChange }: { id: string; value: string; required?: boolean; onChange: (value: string) => void }) {
  const { locale, t } = useI18n();
  const pickerId = `${id}-picker`;
  const placeholder = t("scheduled.flow.dateFormat");
  const label = t("scheduled.flow.chooseDate");
  const formattedValue = value ? formatScheduleDate(value, locale) : "";
  const [draft, setDraft] = useState(formattedValue);
  const [open, setOpen] = useState(false);
  const [viewMonth, setViewMonth] = useState<CalendarMonth>(() => calendarMonth(value));
  const pickerRef = useRef<HTMLDivElement>(null);
  const today = todayIso();
  const days = calendarDays(viewMonth);
  const weekdays = Array.from({ length: 7 }, (_, index) => formatCalendarDate(`2026-09-${String(20 + index).padStart(2, "0")}`, locale, { weekday: "short" }));

  useEffect(() => setDraft(formattedValue), [formattedValue]);
  useEffect(() => {
    if (!open) return;
    const dismissOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !pickerRef.current?.contains(event.target)) setOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", dismissOutside);
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOutside);
      document.removeEventListener("keydown", closeOnEscape);
    };
  }, [open]);

  const commitDraft = (next: string) => {
    const parsed = parseScheduleDate(next, locale);
    if (!next.trim()) {
      setDraft("");
      onChange("");
      return;
    }
    if (!parsed) {
      setDraft(formattedValue);
      return;
    }
    setDraft(formatScheduleDate(parsed, locale));
    setViewMonth(calendarMonth(parsed));
    onChange(parsed);
  };
  const openPicker = () => {
    const parsed = parseScheduleDate(draft, locale) || value;
    setViewMonth(calendarMonth(parsed));
    setOpen(true);
  };
  const chooseDate = (next: string) => {
    setDraft(formatScheduleDate(next, locale));
    setViewMonth(calendarMonth(next));
    onChange(next);
    setOpen(false);
  };
  return <div className="localized-date-input" ref={pickerRef}>
    <div className="schedule-date-control"><input id={id} className="text-input" type="text" lang={locale} inputMode="numeric" autoComplete="off" aria-label={label} title={label} aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? pickerId : undefined} placeholder={placeholder} required={required} value={draft} aria-invalid={draft.trim() && !parseScheduleDate(draft, locale) ? true : undefined} onClick={openPicker} onChange={(event) => { const next = event.target.value; setDraft(next); const parsed = parseScheduleDate(next, locale); if (!next.trim()) onChange(""); else if (parsed) onChange(parsed); }} onBlur={() => commitDraft(draft)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === "ArrowDown" || event.key === " ") { event.preventDefault(); openPicker(); } }} /><button className="schedule-date-trigger" type="button" lang={locale} aria-label={label} title={label} aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? pickerId : undefined} onClick={openPicker}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true" focusable="false"><rect x="4" y="5.5" width="16" height="14" rx="2" /><path d="M8 3.5v4M16 3.5v4M4 9.5h16" /></svg></button></div>
    {open && <div className="schedule-date-picker" id={pickerId} role="dialog" aria-label={label}>
      <div className="schedule-date-picker-header"><strong>{formatCalendarDate(`${viewMonth.year}-${String(viewMonth.month + 1).padStart(2, "0")}-01`, locale, { year: "numeric", month: "long" })}</strong><div className="schedule-date-picker-nav"><button className="schedule-date-nav-button" type="button" aria-label={t("scheduled.flow.previousMonth")} title={t("scheduled.flow.previousMonth")} onClick={() => setViewMonth(shiftCalendarMonth(viewMonth, -1))}>‹</button><button className="schedule-date-nav-button" type="button" aria-label={t("scheduled.flow.nextMonth")} title={t("scheduled.flow.nextMonth")} onClick={() => setViewMonth(shiftCalendarMonth(viewMonth, 1))}>›</button></div></div>
      <div className="schedule-date-weekdays" aria-hidden="true">{weekdays.map((weekday, index) => <span className="schedule-date-weekday" key={`${weekday}-${index}`}>{weekday}</span>)}</div>
      <div className="schedule-date-grid">{days.map((day) => <button className={`schedule-date-day${day.outside ? " outside-month" : ""}${day.iso === value ? " selected" : ""}${day.iso === today ? " today" : ""}`} type="button" data-date={day.iso} aria-label={formatCalendarDate(day.iso, locale, { year: "numeric", month: "long", day: "numeric" })} aria-pressed={day.iso === value} onClick={() => chooseDate(day.iso)} key={day.iso}>{day.day}</button>)}</div>
      <div className="schedule-date-picker-footer"><button className="button small secondary" type="button" onClick={() => chooseDate("")}>{t("scheduled.flow.clearDate")}</button><button className="button small secondary" type="button" onClick={() => chooseDate(today)}>{t("scheduled.flow.today")}</button></div>
    </div>}
  </div>;
}

export function ScheduleInputs({ value, timezones, defaultTimezone, onChange, idPrefix = "schedule" }: { value: ScheduleInput; timezones: string[]; defaultTimezone: string; onChange: (value: ScheduleInput) => void; idPrefix?: string }) {
  const { t } = useI18n();
  const timezoneField = (timezone: string, update: (timezone: string) => void) => <label className="schedule-field schedule-timezone-field"><span>{t("scheduled.flow.timezone")}</span><TimezonePicker id={`${idPrefix}-timezone`} suggestionsId={`${idPrefix}-timezone-suggestions`} value={timezone} timezones={timezones} placeholder={t("config.timezonePlaceholder")} required onChange={update} /></label>;
  const timeField = (time: string, update: (time: string) => void) => <label className="schedule-field schedule-time-field"><span>{t("scheduled.flow.time")}</span><TimePicker id={`${idPrefix}-time`} value={time} hourLabel={t("scheduled.flow.hour")} minuteLabel={t("scheduled.flow.minute")} required onChange={update} /></label>;
  return <div className="schedule-inputs"><label className="schedule-field schedule-kind-field"><span>{t("scheduled.flow.scheduleType")}</span><select className="select-input" value={value.kind} onChange={(event) => onChange(event.target.value === "daily" ? { kind: "daily", time: "00:00", timezone: defaultTimezone } : event.target.value === "periodic" ? { kind: "periodic", every: "1h", first_at: null } : { kind: "once", date: "", time: "", timezone: defaultTimezone })}><option value="once">{t("scheduled.flow.once")}</option><option value="daily">{t("scheduled.flow.daily")}</option><option value="periodic">{t("scheduled.flow.periodic")}</option></select></label>{value.kind === "once" ? <><label className="schedule-field"><span>{t("scheduled.flow.date")}</span><LocalizedDateInput id={`${idPrefix}-date`} value={value.date} required onChange={(date) => onChange({ ...value, date })} /></label>{timeField(value.time, (time) => onChange({ ...value, time }))}{timezoneField(value.timezone, (timezone) => onChange({ ...value, timezone }))}</> : value.kind === "daily" ? <>{timeField(value.time, (time) => onChange({ ...value, time }))}{timezoneField(value.timezone, (timezone) => onChange({ ...value, timezone }))}</> : <label className="schedule-field"><span>{t("scheduled.flow.interval")}</span><input className="text-input" required value={value.every} onChange={(event) => onChange({ ...value, every: event.target.value })} /></label>}</div>;
}
