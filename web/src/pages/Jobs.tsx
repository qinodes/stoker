import { useI18n } from "../i18n/context";
import { clearLocalizedValidity, validateLocalizedForm } from "../form-validation";
import { useEffect, useRef, useState } from "react";
import { EmptyState, JobRow, LiveTime, Pagination, StateBadge } from "../components";
import { isTerminalState, limitUnicode } from "../formatters";
import { JOBS_PAGE_SIZE, pageInfo, useWorkspace } from "../context";
import type { Job, PageInfo } from "../types";

const JOB_STATES = ["DRAFT", "QUEUED", "STARTING", "RUNNING", "CANCELLING", "SUCCEEDED", "FAILED", "CANCELLED", "LOST"];

export function Jobs() {
  const { t, stateLabel } = useI18n();
  const { state, actions } = useWorkspace();
  const query = state.filters.search.toLowerCase();
  const filtered = state.jobs.filter((job) => {
    const matchesSearch = !query || [job.name, job.user, job.cwd, job.id].some((value) => String(value || "").toLowerCase().includes(query));
    return matchesSearch && (!state.filters.user || job.user === state.filters.user) && (!state.filters.state || job.state === state.filters.state);
  });
  const page = pageInfo(filtered, state.pagination.jobs, JOBS_PAGE_SIZE);
  const owners = [...new Set(state.jobs.map((job) => job.user))].sort();
  const terminalCount = state.jobs.filter((job) => isTerminalState(job.state)).length;
  return <>
    <div className="page-heading"><div><h1>{t("jobs.title")}</h1><p>{t("jobs.description")}</p></div></div>
    <div className="toolbar jobs-toolbar"><div className="jobs-toolbar-search"><input className="search-input" id="job-search" type="search" autoComplete="off" spellCheck={false} placeholder={t("jobs.searchPlaceholder")} value={state.filters.search} aria-label={t("jobs.search")} onChange={(event) => actions.setFilter("search", event.target.value)} /><div className="filter-group"><select className="select-input" id="user-filter" aria-label={t("jobs.filterOwner")} value={state.filters.user} onChange={(event) => actions.setFilter("user", event.target.value)}><option value="">{t("jobs.allOwners")}</option>{owners.map((owner) => <option value={owner} key={owner}>{owner}</option>)}</select><select className="select-input" id="state-filter" aria-label={t("jobs.filterState")} value={state.filters.state} onChange={(event) => actions.setFilter("state", event.target.value)}><option value="">{t("jobs.allStates")}</option>{JOB_STATES.map((value) => <option value={value} key={value}>{stateLabel(value)}</option>)}</select></div></div><div className="jobs-toolbar-actions"><button className="button primary" data-action="new-job" type="button" onClick={() => void actions.openJobForm()}>{t("jobs.newButton")}</button><button className="button danger" data-action="clean-jobs" type="button" disabled={!terminalCount} onClick={() => void actions.cleanJobs()}>{t("jobs.cleanHistory")}{terminalCount ? ` (${terminalCount})` : ""}</button></div></div>
    <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{t(filtered.length === 1 ? "jobs.visibleOne" : "jobs.visibleMany", { count: filtered.length })}</h2><p>{t("jobs.serverState")} <LiveTime value={new Date()} timezone={state.timezone?.name} /></p></div></div></div><div className="table-wrap"><table className="data-table jobs-table"><thead><tr><th>{t("jobs.name")}</th><th>{t("jobs.owner")}</th><th>{t("jobs.path")}</th><th>{t("jobs.state")}</th><th>{t("nav.queue")}</th><th>{t("jobs.created")}</th></tr></thead><tbody>{page.items.length ? page.items.map((job) => <JobRow job={job} timezone={state.timezone?.name} onOpen={actions.openJobDetail} key={job.id} />) : <tr><td colSpan={6}><EmptyState icon="○" title={t("jobs.emptyFilters")} /></td></tr>}</tbody></table></div><Pagination kind="jobs" page={page} onPage={(next) => actions.setPage("jobs", next)} /></section>
  </>;
}

export function JobForm() {
  const { t } = useI18n();
  const { state, actions, jobFormOpen } = useWorkspace();
  const [showOwners, setShowOwners] = useState(false);
  const ownerPickerRef = useRef<HTMLDivElement>(null);
  useEffect(() => { if (!jobFormOpen) setShowOwners(false); }, [jobFormOpen]);
  useEffect(() => {
    if (!showOwners) return;
    const closeWhenClickedOutside = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && !ownerPickerRef.current?.contains(target)) setShowOwners(false);
    };
    document.addEventListener("pointerdown", closeWhenClickedOutside);
    return () => document.removeEventListener("pointerdown", closeWhenClickedOutside);
  }, [showOwners]);
  const limits = { name: state.config?.max_job_name_length || 128, user: state.config?.max_job_user_length || 50, description: state.config?.max_job_description_length || 200 };
  const draft = state.jobDraft;
  const owners = [...new Set(state.jobs.map((job) => job.user).filter(Boolean))].filter((owner) => !draft.user || owner.toLowerCase().includes(draft.user.trim().toLowerCase())).sort().slice(0, 50);
  return <div className="dialog-card job-card"><div className="dialog-kicker">{t("jobs.breadcrumb")}</div><div className="job-dialog-heading"><div><h2 id="job-dialog-title">{t("jobs.newTitle")}</h2><p>{t("jobs.newDescription")}</p></div><button className="dialog-close" data-action="close-job-form" type="button" aria-label={t("jobs.closeNew")} onClick={actions.closeJobForm}>×</button></div>
    <form id="new-job-form" noValidate onInputCapture={(event) => clearLocalizedValidity(event.target)} onChangeCapture={(event) => clearLocalizedValidity(event.target)} onSubmit={(event) => { event.preventDefault(); if (validateLocalizedForm(event.currentTarget, t)) void actions.saveJob(); }}>
      <div className="job-form-grid"><div className="job-field"><label htmlFor="job-user">{t("jobs.owner")}</label><div className="job-user-picker" ref={ownerPickerRef}><input className="text-input" id="job-user" maxLength={limits.user} value={draft.user} autoComplete="off" autoCapitalize="off" aria-autocomplete="list" aria-controls="job-user-suggestions" aria-expanded={showOwners && owners.length > 0} required onFocus={() => setShowOwners(true)} onBlur={() => window.setTimeout(() => setShowOwners(false), 120)} onChange={(event) => { setShowOwners(true); actions.updateDraft({ user: event.target.value }); }} /><div className={`job-suggestions${showOwners && owners.length ? " visible" : ""}`} id="job-user-suggestions" role="listbox">{showOwners && owners.length > 0 && <><div className="job-suggestions-label">{t("jobs.knownOwners", { count: owners.length })}</div>{owners.map((owner) => <button className="job-suggestion" type="button" role="option" data-job-user={owner} key={owner} onClick={() => { actions.updateDraft({ user: owner }); setShowOwners(false); }}><span className="suggestion-avatar">{owner.slice(0, 1).toUpperCase()}</span><span>{owner}</span></button>)}</>}</div></div><small className="form-help">{t("jobs.maximum", { count: limits.user })}</small></div><div className="job-field"><label htmlFor="job-name">{t("jobs.name")}</label><input className="text-input" id="job-name" maxLength={limits.name} value={draft.name} autoComplete="off" required onChange={(event) => actions.updateDraft({ name: event.target.value })} /><small className="form-help">{t("jobs.maximum", { count: limits.name })}</small></div></div>
      <div className="job-field"><label htmlFor="job-cwd">{t("jobs.directory")}</label><div className="path-input-row"><input className="text-input mono-input" id="job-cwd" value={draft.cwd} placeholder={t("jobs.directoryPlaceholder")} autoComplete="off" autoCapitalize="off" spellCheck={false} required onChange={(event) => actions.updateDraft({ cwd: event.target.value })} /><button className="button secondary" data-action="browse-directory" type="button" onClick={() => void actions.openDirectoryBrowser(draft.cwd || state.filesystem.roots?.default_path || "")}>{t("jobs.browse")}</button></div><small className="form-help">{t("jobs.directoryHelp")}</small></div>
      <div className="job-field"><label htmlFor="job-command">{t("jobs.command")}</label><textarea className="command-input" id="job-command" rows={4} placeholder="cargo build --release" value={draft.command} required onChange={(event) => actions.updateDraft({ command: event.target.value })} /><small className="form-help">{t("jobs.commandHelp")} <code>stoker create --cmd</code>.</small></div>
      <div className="job-field"><label htmlFor="job-description">{t("jobs.descriptionLabel")} <span className="field-optional">{t("common.optional")}</span></label><textarea className="description-input" id="job-description" rows={4} maxLength={limits.description} placeholder={t("jobs.descriptionPlaceholder")} value={draft.description} onChange={(event) => actions.updateDraft({ description: event.target.value })} /><small className="form-help">{t("jobs.maximum", { count: limits.description })} <span id="job-description-count">{[...draft.description].length}/{limits.description}</span></small></div>
      <div className={`form-feedback${state.filesystem.error ? " invalid" : ""}`} aria-live="polite">{state.filesystem.error}</div><div className="dialog-actions"><button className="button secondary" data-action="close-job-form" type="button" onClick={actions.closeJobForm}>{t("common.cancel")}</button><button className="button primary" type="submit">{t("jobs.createDraft")}</button></div>
    </form>
  </div>;
}

export function DirectoryBrowserDialog() {
  const { t } = useI18n();
  const { actions } = useWorkspace();
  return <div className="directory-browser-shell"><button className="dialog-close" data-action="close-directory-browser" type="button" aria-label={t("jobs.closeDirectoryBrowser")} onClick={actions.closeDirectoryBrowser}>×</button><DirectoryBrowser /></div>;
}

function DirectoryBrowser() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const browser = state.filesystem.current;
  return <div className="fs-browser"><div className="fs-browser-toolbar"><div className="fs-browser-title"><span className="section-kicker">{t("jobs.directoryBrowser")}</span><span className="fs-browser-current-path">{browser?.path || state.filesystem.inputPath}</span></div></div><div className="fs-path-row"><input className="text-input mono-input" id="fs-path" value={state.filesystem.inputPath || browser?.path || ""} onChange={(event) => actions.setFilesystemInputPath(event.target.value)} /><div className="fs-path-actions"><button className="button secondary fs-parent-button" data-action="fs-go-parent" type="button" disabled={!browser?.parent || state.filesystem.loading} onClick={() => { if (browser?.parent) void actions.loadDirectory(browser.parent); }}>{t("jobs.goParent")}</button><button className="button secondary" data-action="open-directory" type="button" onClick={() => void actions.loadDirectory(state.filesystem.inputPath)}>{t("common.open")}</button></div></div>{state.filesystem.error && <div className="form-feedback invalid">{state.filesystem.error}</div>}<div className="fs-directory-list">{state.filesystem.loading ? <EmptyState icon="⌁" title={t("jobs.loadingDirectories")} /> : browser?.directories.length ? browser.directories.map((entry) => <button className="fs-directory-row" type="button" data-directory={entry.path} key={entry.path} onClick={() => void actions.loadDirectory(entry.path)}><span className="fs-folder-glyph">◆</span><span>{entry.name}</span><span className="fs-row-arrow">›</span></button>) : <EmptyState icon="○" title={t("jobs.noDirectories")} />}</div>{browser && <div className="fs-browser-footer"><button className="button small primary" data-action="choose-directory" type="button" onClick={actions.chooseDirectory}>{t("jobs.useDirectory")}</button></div>}</div>;
}

export function JobDetail() {
  const { t } = useI18n();
  const { state, actions, detailEditing } = useWorkspace();
  const job = state.selectedJob;
  const detail = state.selectedJobDetail;
  const [description, setDescription] = useState(job?.description || "");
  useEffect(() => { if (!detailEditing) setDescription(job?.description || ""); }, [detailEditing, job?.description]);
  if (!job) return <div className="job-detail-card"><EmptyState icon="○" title={t("jobs.loading")} /></div>;
  const canCommit = job.state === "DRAFT";
  const canCancel = ["DRAFT", "QUEUED", "STARTING", "RUNNING"].includes(job.state);
  const command = job.command_line || job.command?.join(" ") || "—";
  const directoryStatus = detail?.working_directory_status || (["RUNNING", "STARTING", "CANCELLING"].includes(job.state) ? t("jobs.activeDirectory") : t("jobs.storedDirectory"));
  const timeline: Array<[string, string | null | undefined]> = [[t("jobs.created"), job.created_at], [t("jobs.committed"), job.committed_at], [t("jobs.started"), job.started_at], [t("jobs.finished"), job.finished_at]];
  return <div className="job-detail-card"><div className="job-detail-heading"><div><div className="dialog-kicker">{t("jobs.breadcrumb")}</div><h2 id="job-detail-title">{job.name}</h2><p className="job-detail-id mono"><span>{job.id}</span><button className="copy-id-button" data-action="copy-job-id" data-job-id={job.id} type="button" title={t("jobs.copyId")} aria-label={t("jobs.copyId")} onClick={() => void actions.copyJobId(job.id)}></button></p></div><button className="dialog-close" data-action="close-job-detail" type="button" aria-label={t("jobs.closeDetails")} onClick={actions.closeJobDetail}>×</button></div>
    <div className="job-detail-status"><div><span className="section-kicker">{t("jobs.currentState")}</span><div className="job-detail-state"><StateBadge value={job.state} /></div></div><div className="job-detail-status-meta"><span className="job-detail-directory-status">{directoryStatus}</span></div></div>
    <section className="job-detail-section"><div className="section-kicker">{t("jobs.overview")}</div><div className="job-detail-grid"><div><span className="detail-label">{t("jobs.owner")}</span><strong>{job.user}</strong></div><div><span className="detail-label">{t("jobs.queueOrder")}</span><strong>{job.queue_order ?? "—"}</strong></div><div className="job-detail-wide"><span className="detail-label">{t("jobs.directory")}</span><code>{job.cwd}</code></div><div className="job-detail-wide"><span className="detail-label">{t("jobs.command")}</span><pre className="job-command">{command}</pre></div></div></section>
    <section className="job-detail-section job-description-section"><div className="section-heading-row"><div className="section-kicker">{t("jobs.descriptionLabel")}</div><button className="button small secondary" data-action="edit-description" type="button" onClick={actions.toggleDescriptionEdit}>{detailEditing ? t("common.cancel") : (job.description ? t("common.edit") : t("jobs.addDescription"))}</button></div>{detailEditing ? <form id="description-form" onSubmit={(event) => { event.preventDefault(); void actions.saveDescription(description); }}><textarea className="description-input job-description-editor" id="description-input" maxLength={state.config?.max_job_description_length || 200} rows={5} value={description} onChange={(event) => setDescription(event.target.value)} /><div className="description-editor-meta"><small className="form-help">{[...description].length}/{state.config?.max_job_description_length || 200}</small><div className="description-editor-actions"><button className="button small primary" type="submit">{t("common.save")}</button></div></div></form> : <p className={`job-description-preview${job.description ? "" : " empty"}`}>{limitUnicode(job.description || t("jobs.noDescription"), 1000)}</p>}</section>
    <section className="job-detail-section"><div className="timeline-heading"><div className="section-kicker">{t("jobs.timeline")}</div>{detail?.display_timezone && <span className="job-detail-timezone">{t("jobs.timesTimezone", { timezone: detail.display_timezone })}</span>}</div><div className="job-timeline">{timeline.map(([label, value]) => <div className={`timeline-item${value ? " has-value" : ""}`} key={label}><span className="timeline-dot" aria-hidden="true"></span><span className="timeline-label">{label}</span><span className="timeline-time">{value ? <LiveTime value={value} timezone={state.timezone?.name} /> : t("jobs.notRecorded")}</span></div>)}</div></section>
    <div className="job-detail-actions"><button className="button secondary" data-action="view-job-logs" data-job-id={job.id} onClick={() => void actions.viewJobLogs(job.id)}>{t("jobs.viewLogs")}</button><span className="job-action-spacer"></span>{canCommit && <button className="button primary" data-job-action="commit" data-job-id={job.id} onClick={() => void actions.commitJob(job.id)}>{t("jobs.commit")}</button>}{canCancel && !isTerminalState(job.state) && <button className="button danger" data-job-action="cancel" data-job-id={job.id} onClick={() => void actions.cancelJob(job.id)}>{t("jobs.cancel")}</button>}</div>
  </div>;
}

export type JobsPageInfo = PageInfo<Job>;
