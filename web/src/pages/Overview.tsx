import { EmptyState, LiveTime, Metric, StateBadge } from "../components";
import { shortId } from "../formatters";
import { useWorkspace } from "../context";
import type { Job } from "../types";

const ACTIVE_STATES = ["STARTING", "RUNNING", "CANCELLING"];

export function Overview() {
  const { state, actions } = useWorkspace();
  const status = state.status;
  const active = state.jobs.find((job) => ACTIVE_STATES.includes(job.state));
  const queue = state.queue.jobs.slice(0, 5);
  const counts = status?.counts || { active: 0, queued: 0, draft: 0, succeeded: 0, failed: 0 };
  return <>
    <div className="page-heading">
      <div><div className="eyebrow">Queue control center</div><h1>See what’s running and what’s next.</h1><p>Keep long-running work moving with a clear view of what is running, waiting, and ready to ship.</p></div>
      <button className="button secondary" data-action="refresh" onClick={() => void actions.loadData()}>Refresh workspace <span aria-hidden="true">↻</span></button>
    </div>
    <div className="metrics-grid">
      <Metric label="Running" value={counts.active} note={active?.name || "No active job"} accent="accent-ember" />
      <Metric label="Queued" value={counts.queued} note={state.queue.locked ? "Queue is locked" : "Ready to run"} accent="accent-cyan" />
      <Metric label="Drafts" value={counts.draft} note="Awaiting review" />
      <Metric label="Succeeded" value={counts.succeeded} note="Completed jobs" accent="accent-green" />
      <Metric label="Failed / lost" value={counts.failed} note="Needs attention" />
    </div>
    <div className="content-grid">
      <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>Active job</h2><p>What the scheduler is working on right now</p></div></div><a className="panel-link" href="#jobs">View all jobs →</a></div><div className="active-job"><ActiveJob job={active} onOpen={actions.openJobDetail} timezone={state.timezone?.name} /></div></section>
      <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>Up next</h2><p>Queue execution order</p></div></div><a className="panel-link" href="#queue">Manage queue →</a></div><div className="queue-list"><QueuePreview jobs={queue} onOpen={actions.openJobDetail} /></div></section>
      <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>Recent activity</h2><p>Latest persisted job events</p></div></div><a className="panel-link" href="#jobs">Open jobs →</a></div><div className="activity-list"><Activity jobs={state.jobs} onOpen={actions.openJobDetail} timezone={state.timezone?.name} /></div></section>
      <section className="panel"><div className="notice-panel"><div className="notice-icon">⌘</div><div><div className="section-kicker">Terminal workflow</div><h2>{status?.scheduler.running ? "Scheduler is online" : "Scheduler is stopped"}</h2><p>{status?.scheduler.running ? "Newly committed jobs can be claimed in queue order." : <>Run <code>stoker start</code> in the terminal to process queued work. The browser UI remains available for inspection.</>}</p></div></div></section>
    </div>
  </>;
}

function ActiveJob({ job, onOpen, timezone }: { job?: Job; onOpen: (id: string) => Promise<void>; timezone?: string | null }) {
  if (!job) return <EmptyState icon="○" title="No active job" message="When the scheduler claims work, the current command and owner will appear here." />;
  return <div className="job-hero" data-job-open={job.id} onClick={() => void onOpen(job.id)}><div><h3>{job.name}</h3><p>{job.cwd}</p><div className="job-hero-meta"><span>{job.user}</span><span>{shortId(job.id)}</span><span><LiveTime value={job.started_at || job.created_at} timezone={timezone} /></span></div></div><StateBadge value={job.state} /></div>;
}

function QueuePreview({ jobs, onOpen }: { jobs: Job[]; onOpen: (id: string) => Promise<void> }) {
  if (!jobs.length) return <EmptyState icon="≡" title="Queue is clear" message="Committed jobs will show up here in execution order." />;
  return <>{jobs.map((job, index) => <div className="queue-row" data-job-open={job.id} key={job.id} onClick={() => void onOpen(job.id)}><span className="queue-order">{String(index + 1).padStart(2, "0")}</span><div className="queue-job"><strong>{job.name}</strong><small>{job.user} · {shortId(job.id)}</small></div><span className="queue-state">QUEUED</span></div>)}</>;
}

function Activity({ jobs, onOpen, timezone }: { jobs: Job[]; onOpen: (id: string) => Promise<void>; timezone?: string | null }) {
  const recent = [...jobs].sort((a, b) => new Date(b.finished_at || b.created_at || 0).valueOf() - new Date(a.finished_at || a.created_at || 0).valueOf()).slice(0, 4);
  if (!recent.length) return <EmptyState icon="⌁" title="No activity yet" message="Create a DRAFT job with the CLI to start building your workspace." />;
  return <>{recent.map((job) => <div className="activity-item" data-job-open={job.id} key={job.id} onClick={() => void onOpen(job.id)}><div className="activity-line"></div><div className="activity-copy"><strong>{job.name} · {job.state}</strong><small>{job.user} · <LiveTime value={job.finished_at || job.created_at} timezone={timezone} /></small></div></div>)}</>;
}
