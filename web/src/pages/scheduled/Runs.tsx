import { useEffect } from "react";
import { EmptyState, LiveTime, PageHeading, StateBadge } from "../../components";
import { useWorkspace } from "../../context";
import { useI18n } from "../../i18n/context";
import type { ScheduledRun } from "../../types";

const cancellable = new Set(["QUEUED", "STARTING", "RUNNING", "CANCELLING"]);

export function Runs() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  useEffect(() => { if (!state.scheduled.runs.length) void actions.loadScheduledRuns(); }, [actions, state.scheduled.runs.length]);
  const runs = state.scheduled.runs;
  const selected = state.scheduled.selectedRun;
  return <>
    <PageHeading eyebrow={t("scheduled.runs.eyebrow")} title={t("scheduled.runs.title")} description={t("scheduled.runs.description")} actions={<button className="button secondary" type="button" onClick={() => void actions.loadScheduledRuns()}>{t("scheduled.refresh")}</button>} />
    {state.workspace?.recovery_fence && <RecoveryBanner runs={runs} />}
    <section className="panel"><div className="table-wrap"><table className="data-table"><thead><tr><th>{t("scheduled.runs.flow")}</th><th>{t("scheduled.runs.source")}</th><th>{t("scheduled.status")}</th><th>{t("scheduled.runs.started")}</th><th>{t("scheduled.runs.finished")}</th><th>{t("scheduled.runs.actions")}</th></tr></thead><tbody>{runs.length ? runs.map((run) => <tr key={run.run_id} className="workload-row" onClick={() => void actions.openScheduledRun(run.run_id)}><td><strong>{run.flow_id}</strong><small>{run.run_id}</small></td><td>{run.source}</td><td><StateBadge value={run.state} /></td><td><LiveTime value={run.started_at} timezone={state.timezone?.name} /></td><td><LiveTime value={run.finished_at} timezone={state.timezone?.name} /></td><td>{cancellable.has(run.state) && <button className="button small danger" type="button" onClick={(event) => { event.stopPropagation(); void actions.cancelScheduledRun(run); }}>{t("scheduled.runs.cancel")}</button>}</td></tr>) : <tr><td colSpan={6}><EmptyState compact icon="○" title={t("scheduled.runs.empty")} /></td></tr>}</tbody></table></div></section>
    {selected && <RunDetail run={selected} />}
  </>;
}

function RunDetail({ run }: { run: ScheduledRun }) {
  const { t } = useI18n();
  const { actions } = useWorkspace();
  return <section className="panel scheduled-run-detail"><div className="panel-header"><div className="panel-title"><div><h2>{t("scheduled.runs.detail")}</h2><p>{run.run_id}</p></div></div>{cancellable.has(run.state) && <button className="button danger" type="button" onClick={() => void actions.cancelScheduledRun(run)}>{t("scheduled.runs.cancel")}</button>}</div><div className="table-wrap"><table className="data-table"><thead><tr><th>{t("scheduled.runs.task")}</th><th>{t("scheduled.status")}</th><th>{t("scheduled.runs.attempts")}</th><th>{t("scheduled.runs.failure")}</th><th>{t("scheduled.runs.actions")}</th></tr></thead><tbody>{run.tasks.map((task) => { const latest = task.attempts.at(-1); return <tr key={task.task_id}><td>{task.task_id}</td><td><StateBadge value={task.state} /></td><td>{task.attempt_count}</td><td>{latest?.failure_detail || latest?.failure_kind || "—"}</td><td>{cancellable.has(task.state) && !task.cancel_requested && <button className="button small danger" type="button" onClick={() => void actions.cancelScheduledRunTask(run, task.task_id)}>{t("scheduled.runs.cancelTask")}</button>}</td></tr>; })}</tbody></table></div></section>;
}

function RecoveryBanner({ runs }: { runs: ScheduledRun[] }) {
  const { t } = useI18n();
  const { actions } = useWorkspace();
  const recovering = runs.find((run) => run.state === "LOST");
  return <section className="panel recovery-banner"><div><strong>{t("scheduled.recovery.title")}</strong><p>{t("scheduled.recovery.description")}</p></div><button className="button primary" type="button" disabled={!recovering} onClick={() => recovering && void actions.reconcileScheduledRecovery(recovering.run_id)}>{t("scheduled.recovery.reconcile")}</button></section>;
}
