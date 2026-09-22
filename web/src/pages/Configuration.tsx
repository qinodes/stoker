import { useI18n } from "../i18n/context";
import { PageHeading, EmptyState, LiveTime, Pagination, TimezonePicker } from "../components";
import { SNAPSHOTS_PAGE_SIZE, pageInfo, useWorkspace } from "../context";

export function Configuration() {
  const { t } = useI18n();
  const { state, actions } = useWorkspace();
  const settings = state.settings;
  if (!settings) return <div className="page-loading"><span className="spinner"></span><span>{t("config.loading")}</span></div>;
  const snapshots = settings.snapshots || [];
  const page = pageInfo(snapshots, state.pagination.snapshots, SNAPSHOTS_PAGE_SIZE);
  const current = typeof settings.config.timezone === "string" ? settings.config.timezone : "";
  const configured = state.configurationDraft === null ? current : state.configurationDraft;
  const timezone = settings.effective_timezone || { name: "—", source: "—" };
  const exact = settings.timezones.includes(configured.trim());
  const changed = configured.trim() !== current;
  const feedback = exact
    ? (changed ? t("config.validTimezone") : "")
    : configured ? t("config.chooseSuggestion") : t("config.startTyping");
  return <>
    <PageHeading eyebrow={t("config.breadcrumb")} title={t("config.title")} description={t("config.description")} />
    <div className="config-layout"><section className="panel config-card"><div className="panel-header"><div className="panel-title"><div><h2>{t("config.timezone")}</h2><p>{t("config.timezoneDescription")}</p></div></div></div><div className="config-summary"><div><span className="config-label">{t("config.effectiveTimezone")}</span><strong>{timezone.name}</strong></div><div><span className="config-label">{t("config.source")}</span><span className="config-value">{timezone.source}</span></div><div><span className="config-label">{t("config.value")}</span><span className="config-value">{configured || t("config.unset")}</span></div></div><form className="config-form" id="timezone-form" onSubmit={(event) => { event.preventDefault(); if (exact && changed) void actions.saveTimezone(configured.trim()); }}><label htmlFor="timezone-input">{t("config.setIana")}</label><TimezonePicker id="timezone-input" suggestionsId="timezone-suggestions" value={configured} timezones={settings.timezones} placeholder={t("config.timezonePlaceholder")} onChange={actions.setConfigurationDraft} />{feedback && <div id="timezone-feedback" className={`form-feedback ${exact ? "valid" : configured ? "invalid" : ""}`} aria-live="polite">{feedback}</div>}<div className="form-row"><button className="button primary" id="timezone-set" type="submit" disabled={!exact || !changed}>{t("config.setTimezone")}</button><button className="button secondary" data-action="unset-timezone" type="button" onClick={() => void actions.unsetTimezone()}>{t("config.useSystem")}</button></div><small className="form-help">{t("config.timezoneHelp", { count: settings.timezones.length })}</small></form></section>
      <section className="panel config-card"><div className="panel-header"><div className="panel-title"><div><h2>{t("config.snapshots")}</h2><p>{t("config.snapshotsDescription")}</p></div></div><button className="button small secondary" data-action="create-snapshot" type="button" onClick={() => void actions.createSnapshot()}>{t("config.createSnapshot")}</button></div><div className="snapshot-list">{page.items.length ? page.items.map((snapshot) => <div className="snapshot-row" key={snapshot.path}><div className="snapshot-copy"><strong>{snapshot.created_at ? <LiveTime value={snapshot.created_at} timezone={state.timezone?.name} /> : t("config.invalidSnapshot")}</strong><small>{snapshot.reason || snapshot.error || snapshot.path}</small></div><div className="snapshot-actions">{snapshot.valid ? <button className="button small secondary" type="button" data-restore-path={snapshot.path} onClick={() => void actions.restoreSnapshot(snapshot.path)}>{t("config.restore")}</button> : <span className="state-badge state-failed">{t("common.invalid")}</span>}</div></div>) : <EmptyState compact title={t("config.noSnapshots")} message={t("config.snapshotHelp")} />}</div><Pagination kind="snapshots" page={page} onPage={(next) => actions.setPage("snapshots", next)} /><div className="config-paths"><span>{t("config.pathLabel")} <code>{settings.config_path}</code></span><span>{t("config.snapshotsLabel")} <code>{settings.snapshot_dir}</code></span></div></section></div>
    <section className="panel config-card"><div className="panel-header"><div className="panel-title"><div><h2>{t("config.currentConfig")}</h2><p>{t("config.equivalent")} <code>stoker config show</code>.</p></div></div></div><pre className="config-json">{JSON.stringify(settings.config, null, 2)}</pre></section>
  </>;
}
