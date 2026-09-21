import { EmptyState, LiveTime, Metric, PageHeading, StateBadge } from "../../components";
import { useWorkspace } from "../../context";
import { useI18n } from "../../i18n/context";

export function ScheduledOverview() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const overview = state.scheduled.overview;
  const capacity = overview?.capacity;
  return <>
    <PageHeading eyebrow={t("scheduled.overview.eyebrow")} title={t("scheduled.overview.title")} description={t("scheduled.overview.description")} actions={<button className="button secondary" type="button" onClick={() => void actions.loadData()}>{t("scheduled.refresh")}</button>} />
    <div className="metrics-grid scheduled-metrics">
      <Metric label={t("scheduled.capacity")} value={`${capacity?.active_attempts || 0}/${capacity?.max_concurrency || 0}`} note={t("scheduled.activeAttempts")} accent="accent-cyan" />
      <Metric label={t("scheduled.activeRuns")} value={overview?.active_runs.length || 0} note={t("scheduled.now")} accent="accent-ember" />
      <Metric label={t("scheduled.nextOccurrences")} value={overview?.next_occurrences.length || 0} note={t("scheduled.queuedOccurrences")} />
      <Metric label={t("scheduled.recentFailures")} value={overview?.recent_failures.length || 0} note={t("scheduled.attention")} />
    </div>
    <div className="content-grid scheduled-overview-grid">
      <RunPanel title={t("scheduled.activeRuns")} items={overview?.active_runs || []} empty={t("scheduled.noActiveRuns")} />
      <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{t("scheduled.nextOccurrences")}</h2><p>{t("scheduled.nextDescription")}</p></div></div></div><div className="occurrence-list">{overview?.next_occurrences.length ? overview.next_occurrences.slice(0, 6).map((item) => <div className="occurrence-row" key={item.occurrence_id}><code>{item.flow_id}</code><LiveTime value={item.due_at} timezone={state.timezone?.name} /></div>) : <EmptyState compact icon="○" title={t("scheduled.noOccurrences")} />}</div></section>
      <RunPanel title={t("scheduled.recentFailures")} items={overview?.recent_failures || []} empty={t("scheduled.noFailures")} />
    </div>
  </>;
}

function RunPanel({ title, items, empty }: { title: string; items: Array<{ run_id: string; flow_id: string; state: string; started_at?: string | null; finished_at?: string | null }>; empty: string }) {
  const { t } = useI18n();
  return <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{title}</h2><p>{t("scheduled.flowRuns")}</p></div></div></div><div className="occurrence-list">{items.length ? items.slice(0, 6).map((run) => <div className="occurrence-row" key={run.run_id}><div><code>{run.flow_id}</code><small>{run.run_id}</small></div><StateBadge value={run.state} /></div>) : <EmptyState compact icon="○" title={empty} />}</div></section>;
}
