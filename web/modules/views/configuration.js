import { pageHeading, pagination } from "../components.js";
import { escapeAttribute, escapeHtml, liveTime } from "../formatters.js";
import { SNAPSHOTS_PAGE_SIZE, pageInfo } from "../state.js";

export function renderConfiguration(state) {
  const settings = state.settings;
  if (!settings) return '<div class="page-loading"><span class="spinner"></span><span>Loading configuration…</span></div>';
  const snapshots = settings.snapshots || [];
  const page = pageInfo(snapshots, state.pagination.snapshots, SNAPSHOTS_PAGE_SIZE);
  state.pagination.snapshots = page.page;
  const current = settings.config.timezone || "";
  const configured = state.configurationDraft === null ? current : state.configurationDraft;
  const timezone = settings.effective_timezone || { name: "—", source: "—" };
  const snapshotRows = page.items.map((snapshot) => `<div class="snapshot-row"><div class="snapshot-copy"><strong>${snapshot.created_at ? liveTime(snapshot.created_at, state.timezone?.name) : "Invalid snapshot"}</strong><small>${escapeHtml(snapshot.reason || snapshot.error || snapshot.path)}</small></div><div class="snapshot-actions">${snapshot.valid ? `<button class="button small secondary" type="button" data-restore-path="${escapeAttribute(snapshot.path)}">Restore</button>` : '<span class="state-badge state-failed">Invalid</span>'}</div></div>`).join("") || '<div class="empty-state compact"><strong>No snapshots yet</strong><p>Save a manual snapshot before making a configuration change.</p></div>';
  return `
    <div class="page-heading"><div><div class="eyebrow">Workspace / Configuration</div><h1>Workspace settings</h1><p>Manage the same timezone and configuration snapshots available from the CLI.</p></div><button class="button secondary" data-action="refresh">Refresh configuration <span aria-hidden="true">↻</span></button></div>
    <div class="config-layout">
      <section class="panel config-card"><div class="panel-header"><div class="panel-title"><div><h2>Timezone</h2><p>Used when displaying timestamps in the UI and CLI.</p></div></div></div><div class="config-summary"><div><span class="config-label">Effective timezone</span><strong>${escapeHtml(timezone.name)}</strong></div><div><span class="config-label">Source</span><span class="config-value">${escapeHtml(timezone.source)}</span></div><div><span class="config-label">Configured value</span><span class="config-value">${escapeHtml(configured || "Not set · follows system")}</span></div></div><form class="config-form" id="timezone-form"><label for="timezone-input">Set IANA timezone</label><div class="timezone-picker"><input class="text-input" id="timezone-input" name="timezone" value="${escapeAttribute(configured)}" placeholder="Type Tokyo, Taipei, UTC…" autocomplete="off" role="combobox" aria-autocomplete="list" aria-controls="timezone-suggestions" aria-expanded="false"><div class="timezone-suggestions" id="timezone-suggestions" role="listbox"></div></div><div id="timezone-feedback" class="form-feedback" aria-live="polite">${configured === current ? "Current timezone is already selected." : ""}</div><div class="form-row"><button class="button primary" id="timezone-set" type="submit" ${configured === current ? "disabled" : ""}>Set timezone</button><button class="button secondary" data-action="unset-timezone" type="button">Use system</button></div><small class="form-help">Type to search ${settings.timezones.length} IANA timezones, then click a suggestion or press Enter.</small></form></section>
      <section class="panel config-card"><div class="panel-header"><div class="panel-title"><div><h2>Configuration snapshots</h2><p>Restore a known configuration without touching jobs.</p></div></div><button class="button small secondary" data-action="create-snapshot" type="button">Create snapshot</button></div><div class="snapshot-list">${snapshotRows}</div>${pagination("snapshots", page)}<div class="config-paths"><span>Config: <code>${escapeHtml(settings.config_path)}</code></span><span>Snapshots: <code>${escapeHtml(settings.snapshot_dir)}</code></span></div></section>
    </div>
    <section class="panel config-card"><div class="panel-header"><div class="panel-title"><div><h2>Current config</h2><p>Equivalent to <code>stoker config show</code>.</p></div></div></div><pre class="config-json">${escapeHtml(JSON.stringify(settings.config, null, 2))}</pre></section>`;
}
