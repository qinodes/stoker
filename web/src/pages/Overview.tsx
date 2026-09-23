import { useI18n } from "../i18n/context";
import { EmptyState, LiveTime, Metric, StateBadge } from "../components";
import { shortId } from "../formatters";
import { useWorkspace } from "../context";
import type { Job } from "../types";

const ACTIVE_STATES = ["STARTING", "RUNNING", "CANCELLING"];

export function Overview() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const status = state.status;
  const active = state.jobs.find((job) => ACTIVE_STATES.includes(job.state));
  const queue = state.queue.jobs.slice(0, 5);
  const counts = status?.counts || { active: 0, queued: 0, draft: 0, succeeded: 0, failed: 0 };
  return <>
    <div className="page-heading"><div><h1>{t("overview.title")}</h1><p>{t("overview.description")}</p></div></div>
    <div className="metrics-grid">
      <Metric label={t("overview.running")} value={counts.active} note={active?.name || t("overview.noActive")} accent="accent-ember" />
      <Metric label={t("overview.queued")} value={counts.queued} note={status?.disk_pressure ? t("overview.diskWait") : state.queue.locked ? t("queue.isLocked") : t("overview.ready")} accent="accent-cyan" />
      <Metric label={t("overview.drafts")} value={counts.draft} note={t("overview.awaitingReview")} />
      <Metric label={t("overview.succeeded")} value={counts.succeeded} note={t("overview.completed")} accent="accent-green" />
      <Metric label={t("overview.failedLost")} value={counts.failed} note={t("overview.attention")} />
    </div>
    <div className="content-grid">
      <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{t("overview.activeJob")}</h2><p>{t("overview.activeDescription")}</p></div></div><a className="panel-link" href="#jobs">{t("overview.viewAll")}</a></div><div className="active-job"><ActiveJob job={active} onOpen={actions.openJobDetail} timezone={state.timezone?.name} /></div></section>
      <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{t("overview.upNext")}</h2><p>{t("overview.queueOrder")}</p></div></div><a className="panel-link" href="#queue">{t("overview.manageQueue")}</a></div><div className="queue-list"><QueuePreview jobs={queue} onOpen={actions.openJobDetail} /></div></section>
      <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{t("overview.activity")}</h2><p>{t("overview.activityDescription")}</p></div></div><a className="panel-link" href="#jobs">{t("overview.openJobs")}</a></div><div className="activity-list"><Activity jobs={state.jobs} onOpen={actions.openJobDetail} timezone={state.timezone?.name} /></div></section>
      <section className="panel"><div className="notice-panel"><div className="notice-icon">⌘</div><div><div className="section-kicker">{t("overview.terminal")}</div><h2>{status?.scheduler.running ? t("overview.online") : t("overview.stopped")}</h2><p>{status?.scheduler.running ? t("overview.claimReady") : <>{t("overview.runPrefix")} <code>stoker start</code> {t("overview.runSuffix")}</>}</p></div></div></section>
    </div>
  </>;
}

function ActiveJob({ job, onOpen, timezone }: { job?: Job; onOpen: (id: string) => Promise<void>; timezone?: string | null }) {
  const { t } = useI18n();
  if (!job) return <EmptyState icon="○" title={t("overview.noActive")} message={t("overview.activeEmpty")} />;
  return <div className="job-hero" data-job-open={job.id} onClick={() => void onOpen(job.id)}><div><h3>{job.name}</h3><p>{job.cwd}</p><div className="job-hero-meta"><span>{job.user}</span><span>{shortId(job.id)}</span><span><LiveTime value={job.started_at || job.created_at} timezone={timezone} /></span></div></div><StateBadge value={job.state} /></div>;
}

function QueuePreview({ jobs, onOpen }: { jobs: Job[]; onOpen: (id: string) => Promise<void> }) {
  const { t, stateLabel } = useI18n();
  if (!jobs.length) return <EmptyState icon="≡" title={t("overview.queueClear")} message={t("overview.queueEmpty")} />;
  return <>{jobs.map((job, index) => <div className="queue-row" data-job-open={job.id} key={job.id} onClick={() => void onOpen(job.id)}><span className="queue-order">{String(index + 1).padStart(2, "0")}</span><div className="queue-job"><strong>{job.name}</strong><small>{job.user} · {shortId(job.id)}</small></div><span className="queue-state">{stateLabel("QUEUED")}</span></div>)}</>;
}

function Activity({ jobs, onOpen, timezone }: { jobs: Job[]; onOpen: (id: string) => Promise<void>; timezone?: string | null }) {
  const { t, stateLabel } = useI18n();
  const recent = [...jobs].sort((a, b) => new Date(b.finished_at || b.created_at || 0).valueOf() - new Date(a.finished_at || a.created_at || 0).valueOf()).slice(0, 4);
  if (!recent.length) return <EmptyState icon="⌁" title={t("overview.noActivity")} message={t("overview.activityEmpty")} />;
  return <>{recent.map((job) => <div className="activity-item" data-job-open={job.id} key={job.id} onClick={() => void onOpen(job.id)}><div className="activity-line"></div><div className="activity-copy"><strong>{job.name} · {stateLabel(job.state)}</strong><small>{job.user} · <LiveTime value={job.finished_at || job.created_at} timezone={timezone} /></small></div></div>)}</>;
}
