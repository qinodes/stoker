import { EmptyState, LiveTime, Metric, PageHeading, StateBadge } from "../../components";
import { useWorkspace } from "../../context";
import { useI18n } from "../../i18n/context";

export function ScheduledOverview() {
  const { t } = useI18n();
  const { state } = useWorkspace();
  const overview = state.scheduled.overview;
  const capacity = overview?.capacity;
  return <>
    <PageHeading title={t("scheduled.overview.title")} description={t("scheduled.overview.description")} />
    <div className="metrics-grid scheduled-metrics">
      <Metric label={t("scheduled.capacity")} value={`${capacity?.active_attempts || 0}/${capacity?.max_concurrency || 0}`} note={t("scheduled.activeAttempts")} accent="accent-cyan" />
      <Metric label={t("scheduled.flows")} value={overview?.flow_count || 0} note={t("scheduled.flowSummary", { live: overview?.live_flow_count || 0, draft: overview?.draft_flow_count || 0 })} accent="accent-ember" />
    </div>
    <div className="content-grid scheduled-overview-grid">
      <RunPanel title={t("scheduled.activeRuns")} description={t("scheduled.activeRunsDescription")} items={overview?.active_runs || []} empty={t("scheduled.noActiveRuns")} timezone={state.timezone?.name} />
      <RunPanel title={t("scheduled.recentFailures")} description={t("scheduled.recentFailuresDescription")} items={overview?.recent_failures || []} empty={t("scheduled.noFailures")} timezone={state.timezone?.name} />
      <RunPanel title={t("scheduled.recovery.overviewTitle")} description={t("scheduled.recovery.overviewDescription")} items={overview?.recovering_runs || []} empty={overview?.recovery_fence ? t("scheduled.recovery.fenceActive") : t("scheduled.recovery.none")} timezone={state.timezone?.name} warning={Boolean(overview?.recovery_fence)} />
    </div>
  </>;
}

function RunPanel({ title, description, items, empty, timezone, warning = false }: { title: string; description: string; items: Array<{ run_id: string; flow_id: string; state: string; started_at?: string | null; finished_at?: string | null }>; empty: string; timezone?: string; warning?: boolean }) {
  return <section className={`panel overview-run-panel${warning ? " recovery-warning" : ""}`}><div className="panel-header"><div className="panel-title"><div><h2>{title}<span className="overview-panel-count">{items.length}</span></h2><p>{description}</p></div></div></div><div className="occurrence-list">{items.length ? items.slice(0, 6).map((run) => <div className="occurrence-row" key={run.run_id}><div><code>{run.flow_id}</code><small>{run.run_id}</small></div><div className="overview-run-state"><StateBadge value={run.state} /><LiveTime value={run.finished_at || run.started_at} timezone={timezone} /></div></div>) : <EmptyState compact icon="○" title={empty} />}</div></section>;
}
