import { PageHeading, StateBadge } from "../components";
import { shortId } from "../formatters";
import { useWorkspace } from "../context";

const MAX_LOG_KB = 256;

export function Logs() {
  const { state, actions } = useWorkspace();
  const selectedJob = state.jobs.find((job) => job.id === state.logs.jobId);
  const query = state.logs.search.trim().toLowerCase();
  const matching = state.jobs.filter((job) => !query || [job.name, job.user, job.id, job.cwd].some((value) => String(value || "").toLowerCase().includes(query)));
  const choices = selectedJob && !matching.some((job) => job.id === selectedJob.id) ? [selectedJob, ...matching] : matching;
  const data = state.logs.data;
  const stream = state.logs.stream === "stderr" ? "stderr" : "stdout";
  const available = data ? Boolean(data[`${stream}_available` as "stdout_available" | "stderr_available"]) : false;
  const output = selectedJob && data && available ? data[stream] || "(No output written yet.)" : "Select a job to inspect its output.";
  const truncated = data ? Boolean(data[`${stream}_truncated` as "stdout_truncated" | "stderr_truncated"]) : false;
  const message = state.logs.error || data?.message || (!state.jobs.length ? "No jobs are available yet. Add a job with the CLI first." : (!available && selectedJob ? `No ${stream} log is available for this job yet.` : ""));
  return <>
    <PageHeading eyebrow="Workspace / Logs" title="Job logs" description="Choose a job, then switch between stdout and stderr. Logs are shown as plain text and are refreshed automatically every 2 seconds." actions={<button className="button secondary" data-action="refresh" onClick={() => void actions.loadData()}>Refresh logs <span aria-hidden="true">↻</span></button>} />
    <section className="panel logs-panel"><div className="logs-toolbar"><label htmlFor="log-job-select">Job</label><input className="search-input log-job-search" id="log-job-search" type="search" autoComplete="off" spellCheck={false} value={state.logs.search} placeholder="Filter jobs" aria-label="Filter jobs" onChange={(event) => actions.setLogSearch(event.target.value)} /><select className="select-input log-job-select" id="log-job-select" aria-label="Choose a job" value={state.logs.jobId} onChange={(event) => void actions.selectLogJob(event.target.value)}><option value="">Choose a job…</option>{choices.length ? choices.map((job) => <option value={job.id} key={job.id}>{job.name} · {shortId(job.id)} · {job.state}</option>) : <option value="">No matching jobs</option>}</select><div className="log-tabs" role="tablist" aria-label="Log stream"><button className={`log-tab${stream === "stdout" ? " active" : ""}`} type="button" data-log-stream="stdout" role="tab" aria-selected={stream === "stdout"} onClick={() => actions.setLogStream("stdout")}>stdout</button><button className={`log-tab${stream === "stderr" ? " active" : ""}`} type="button" data-log-stream="stderr" role="tab" aria-selected={stream === "stderr"} onClick={() => actions.setLogStream("stderr")}>stderr</button></div></div>{selectedJob && <div className="log-context"><div><strong>{selectedJob.name}</strong><small>{selectedJob.user} · {selectedJob.cwd}</small></div><StateBadge value={selectedJob.state} /></div>}{message && <div className="log-message"><span aria-hidden="true">ⓘ</span><span>{message}</span></div>}<pre className="log-output" aria-live="polite">{output}</pre>{truncated && <div className="log-footnote">Showing the latest {MAX_LOG_KB} KB of {stream} output.</div>}</section>
  </>;
}
