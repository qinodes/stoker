import { useI18n } from "../i18n/context";
import { PageHeading, StateBadge } from "../components";
import { shortId } from "../formatters";
import { useWorkspace } from "../context";

const MAX_LOG_KB = 256;

export function Logs() {
  const { t, stateLabel } = useI18n();
  const { state, actions } = useWorkspace();
  const selectedJob = state.jobs.find((job) => job.id === state.logs.jobId);
  const query = state.logs.search.trim().toLowerCase();
  const matching = state.jobs.filter((job) => !query || [job.name, job.user, job.id, job.cwd].some((value) => String(value || "").toLowerCase().includes(query)));
  const choices = selectedJob && !matching.some((job) => job.id === selectedJob.id) ? [selectedJob, ...matching] : matching;
  const data = state.logs.data;
  const stream = state.logs.stream === "stderr" ? "stderr" : "stdout";
  const available = data ? Boolean(data[`${stream}_available` as "stdout_available" | "stderr_available"]) : false;
  const output = selectedJob && data && available ? data[stream] || t("logs.emptyOutput") : t("logs.selectOutput");
  const truncated = data ? Boolean(data[`${stream}_truncated` as "stdout_truncated" | "stderr_truncated"]) : false;
  const captureError = stream === "stdout" ? data?.stdout_capture_error : data?.stderr_capture_error;
  const message = state.logs.error || data?.message || (captureError ? t("logs.degraded", { stream, error: captureError }) : "") || (!state.jobs.length ? t("logs.noJobs") : (!available && selectedJob ? t("logs.unavailable", { stream }) : ""));
  return <>
    <PageHeading title={t("logs.title")} description={t("logs.description")} />
    <section className="panel logs-panel"><div className="logs-toolbar"><label htmlFor="log-job-select">{t("logs.job")}</label><input className="search-input log-job-search" id="log-job-search" type="search" autoComplete="off" spellCheck={false} value={state.logs.search} placeholder={t("logs.filter")} aria-label={t("logs.filter")} onChange={(event) => actions.setLogSearch(event.target.value)} /><select className="select-input log-job-select" id="log-job-select" aria-label={t("logs.choose")} value={state.logs.jobId} onChange={(event) => void actions.selectLogJob(event.target.value)}><option value="">{t("logs.choosePlaceholder")}</option>{choices.length ? choices.map((job) => <option value={job.id} key={job.id}>{job.name} · {shortId(job.id)} · {stateLabel(job.state)}</option>) : <option value="">{t("logs.noMatching")}</option>}</select><div className="log-tabs" role="tablist" aria-label={t("logs.stream")}><button className={`log-tab${stream === "stdout" ? " active" : ""}`} type="button" data-log-stream="stdout" role="tab" aria-selected={stream === "stdout"} onClick={() => actions.setLogStream("stdout")}>stdout</button><button className={`log-tab${stream === "stderr" ? " active" : ""}`} type="button" data-log-stream="stderr" role="tab" aria-selected={stream === "stderr"} onClick={() => actions.setLogStream("stderr")}>stderr</button></div></div>{selectedJob && <div className="log-context"><div><strong>{selectedJob.name}</strong><small>{selectedJob.user} · {selectedJob.cwd}</small></div><StateBadge value={selectedJob.state} /></div>}{message && <div className="log-message"><span aria-hidden="true">ⓘ</span><span>{message}</span></div>}<pre className="log-output" aria-live="polite">{output}</pre>{truncated && <div className="log-footnote">{t("logs.latest", { count: MAX_LOG_KB, stream })}</div>}</section>
  </>;
}
