import { useEffect } from "react";
import { PageHeading, StateBadge } from "../../components";
import { useWorkspace } from "../../context";
import { useI18n } from "../../i18n/context";

const MAX_LOG_KB = 256;

export function ScheduledLogs() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const scheduled = state.scheduled;
  useEffect(() => { if (!scheduled.runs.length) void actions.loadScheduledRuns(); }, [actions, scheduled.runs.length]);
  const run = scheduled.runs.find((item) => item.run_id === scheduled.logs.runId);
  const task = run?.tasks.find((item) => item.task_id === scheduled.logs.taskId);
  const attempt = task?.attempts.find((item) => item.number === scheduled.logs.attempt);
  const stream = scheduled.logs.stream;
  const data = scheduled.logs.data;
  const available = data ? data[`${stream}_available` as "stdout_available" | "stderr_available"] : false;
  const output = attempt && data && available ? data[stream] || t("logs.emptyOutput") : t("scheduled.logs.selectOutput");
  const truncated = data ? data[`${stream}_truncated` as "stdout_truncated" | "stderr_truncated"] : false;
  const captureError = stream === "stdout" ? data?.stdout_capture_error : data?.stderr_capture_error;
  const message = scheduled.logs.error || data?.message || captureError || (attempt && !available ? t("logs.unavailable", { stream }) : "");
  return <>
    <PageHeading eyebrow={t("scheduled.logs.eyebrow")} title={t("scheduled.logs.title")} description={t("scheduled.logs.description")} />
    <section className="panel logs-panel"><div className="logs-toolbar"><label htmlFor="scheduled-log-run">{t("scheduled.logs.run")}</label><select className="select-input" id="scheduled-log-run" value={scheduled.logs.runId} onChange={(event) => void actions.selectScheduledLogTarget(event.target.value, "", null)}><option value="">{t("scheduled.logs.chooseRun")}</option>{scheduled.runs.map((item) => <option key={item.run_id} value={item.run_id}>{item.flow_id} · {item.run_id}</option>)}</select><label htmlFor="scheduled-log-task">{t("scheduled.runs.task")}</label><select className="select-input" id="scheduled-log-task" value={scheduled.logs.taskId} disabled={!run} onChange={(event) => void actions.selectScheduledLogTarget(run?.run_id || "", event.target.value, null)}><option value="">{t("scheduled.logs.chooseTask")}</option>{run?.tasks.map((item) => <option key={item.task_id} value={item.task_id}>{item.task_id} · {item.state}</option>)}</select><label htmlFor="scheduled-log-attempt">{t("scheduled.logs.attempt")}</label><select className="select-input" id="scheduled-log-attempt" value={scheduled.logs.attempt ?? ""} disabled={!task} onChange={(event) => void actions.selectScheduledLogTarget(run?.run_id || "", task?.task_id || "", event.target.value ? Number(event.target.value) : null)}><option value="">{t("scheduled.logs.chooseAttempt")}</option>{task?.attempts.map((item) => <option key={item.attempt_id} value={item.number}>#{item.number} · {item.state}</option>)}</select><div className="log-tabs" role="tablist" aria-label={t("logs.stream")}><button className={`log-tab${stream === "stdout" ? " active" : ""}`} type="button" onClick={() => actions.setScheduledLogStream("stdout")}>stdout</button><button className={`log-tab${stream === "stderr" ? " active" : ""}`} type="button" onClick={() => actions.setScheduledLogStream("stderr")}>stderr</button></div></div>
      {attempt && <div className="log-context"><div><strong>{run?.flow_id}</strong><small>{task?.task_id} · #{attempt.number}</small></div><StateBadge value={attempt.state} /></div>}{message && <div className="log-message"><span aria-hidden="true">ⓘ</span><span>{message}</span></div>}<pre className="log-output" aria-live="polite">{output}</pre>{truncated && <div className="log-footnote">{t("logs.latest", { count: MAX_LOG_KB, stream })}</div>}</section>
  </>;
}
