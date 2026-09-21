import { useI18n } from "../i18n/context";
import { EmptyState, PageHeading, StateBadge } from "../components";
import { shortId } from "../formatters";
import { useWorkspace } from "../context";

export function Queue() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const jobs = state.queue.jobs || [];
  const locked = Boolean(state.queue.locked);
  return <>
    <PageHeading eyebrow={t("queue.breadcrumb")} title={t("queue.title")} description={t("queue.description")} actions={<><button className="button primary" data-queue-lock="true" disabled={locked} onClick={() => void actions.queueLock(true)}>{t("queue.lock")}</button><button className="button secondary" data-queue-lock="false" disabled={!locked} onClick={() => void actions.queueLock(false)}>{t("queue.unlock")}</button></>} />
    {locked ? <div className="queue-lock-banner"><div><strong>{t("queue.isLocked")}</strong><small>{t("queue.lockHelp")}</small></div><span aria-hidden="true">🔒</span></div> : <div className="queue-unlock-banner"><div><strong>{t("queue.isUnlocked")}</strong><small>{t("queue.unlockHelp")}</small></div><span aria-hidden="true">↕</span></div>}
    <section className="panel"><div className="panel-header"><div className="panel-title"><div><h2>{t(jobs.length === 1 ? "queue.countOne" : "queue.countMany", { count: jobs.length })}</h2><p>{t("queue.serverOrder")}</p></div></div></div><div className="queue-table"><table className="data-table queue-order-table"><thead><tr><th>#</th><th>{t("jobs.name")}</th><th>{t("jobs.owner")}</th><th>{t("jobs.path")}</th><th>{t("jobs.state")}</th><th>{t("queue.move")}</th></tr></thead><tbody>{jobs.length ? jobs.map((job, index) => <tr key={job.id}><td className="mono">{index + 1}</td><td><div className="job-name"><strong>{job.name}</strong><small>{shortId(job.id)}</small></div></td><td>{job.user}</td><td className="path-cell" title={job.cwd}>{job.cwd}</td><td><StateBadge value={job.state} /></td><td><div className="move-controls"><button className="move-button" type="button" data-queue-move={job.id} data-target-order={index} aria-label={t("queue.moveUp", { name: job.name })} disabled={!locked || index === 0} onClick={() => void actions.moveQueueJob(job.id, index)}>↑</button><button className="move-button" type="button" data-queue-move={job.id} data-target-order={index + 2} aria-label={t("queue.moveDown", { name: job.name })} disabled={!locked || index === jobs.length - 1} onClick={() => void actions.moveQueueJob(job.id, index + 2)}>↓</button></div></td></tr>) : <tr><td colSpan={6}><EmptyState icon="○" title={t("queue.empty")} /></td></tr>}</tbody></table></div></section>
  </>;
}
