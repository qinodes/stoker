import { useI18n } from "../i18n/context";
import { useEffect, useRef, useState } from "react";
import { PageHeading } from "../components";
import { useWorkspace } from "../context";
import { formatPolicyValue, policyInputIsValid, policyRouteKey } from "../policy";
import type { StaticKey } from "../i18n/messages.ts";
import type { PolicyResponse } from "../types";

type PolicySection = "log" | "runtime";
type PolicyKey = keyof PolicyResponse["log"] | keyof PolicyResponse["runtime"];

interface FieldDefinition {
  section: PolicySection;
  key: PolicyKey;
  label: StaticKey;
  description: StaticKey;
  unit: "MB" | "jobs" | "milliseconds";
}

const LOG_FIELDS: FieldDefinition[] = [
  { section: "log", key: "max_bytes_per_job", label: "policy.logPerJob", description: "policy.logPerJobDescription", unit: "MB" },
  { section: "log", key: "segment_bytes", label: "policy.segment", description: "policy.segmentDescription", unit: "MB" },
  { section: "log", key: "max_bytes_total", label: "policy.total", description: "policy.totalDescription", unit: "MB" },
  { section: "log", key: "retention_jobs", label: "policy.retention", description: "policy.retentionDescription", unit: "jobs" },
  { section: "log", key: "disk_reserve_bytes", label: "policy.reserve", description: "policy.reserveDescription", unit: "MB" },
];

const RUNTIME_FIELDS: FieldDefinition[] = [
  { section: "runtime", key: "termination_grace_ms", label: "policy.grace", description: "policy.graceDescription", unit: "milliseconds" },
  { section: "runtime", key: "startup_timeout_ms", label: "policy.startup", description: "policy.startupDescription", unit: "milliseconds" },
  { section: "runtime", key: "max_runtime_ms", label: "policy.maxRuntime", description: "policy.maxRuntimeDescription", unit: "milliseconds" },
];

const FIELDS = [...LOG_FIELDS, ...RUNTIME_FIELDS];

export function Policy() {
  const { t, stateLabel } = useI18n();
  const { state, actions } = useWorkspace();
  const policy = state.policy;
  const [draft, setDraft] = useState<Record<string, string>>({});
  const lastPolicy = useRef<PolicyResponse | null>(null);
  useEffect(() => {
    if (!policy) return;
    const previousPolicy = lastPolicy.current;
    lastPolicy.current = policy;
    setDraft((current) => {
      const next = { ...current };
      for (const field of FIELDS) {
        const previous = previousPolicy ? String(readValue(previousPolicy, field) ?? "") : undefined;
        if (!(field.key in next) || next[field.key] === previous) {
          next[field.key] = String(readValue(policy, field) ?? "");
        }
      }
      return next;
    });
  }, [policy]);

  if (!policy) return <div className="page-loading"><span className="spinner"></span><span>{t("policy.loading")}</span></div>;

  const update = async (field: FieldDefinition) => {
    const text = draft[field.key] || "";
    if (!policyInputIsValid(text, field.key === "retention_jobs")) return;
    const value = Number(text);
    if (await actions.savePolicy(policyKey(field), value)) {
      setDraft((current) => ({ ...current, [field.key]: String(value) }));
    }
  };

  const reset = async (field: FieldDefinition) => {
    if (await actions.unsetPolicy(policyKey(field))) {
      setDraft((current) => ({ ...current, [field.key]: String(readDefault(policy, field) ?? "") }));
    }
  };

  return <>
    <PageHeading eyebrow={t("policy.breadcrumb")} title={t("policy.title")} description={t("policy.description")} actions={<button className="button secondary" data-action="refresh-policy" onClick={() => void actions.loadData()}>{t("policy.refresh")} <span aria-hidden="true">↻</span></button>} />
    <section className={`panel policy-gate${policy.can_update ? " ready" : " blocked"}`} data-policy-gate>
      <div className="policy-gate-copy"><div className="section-kicker">{t("policy.protection")}</div><h2>{policy.can_update ? t("policy.available") : t("policy.paused")}</h2><p>{policy.can_update ? t("policy.readyDescription") : policy.blocked_reason}</p>{policy.active_jobs.length > 0 && <div className="policy-active-jobs"><strong>{t("policy.activeJobs")}</strong>{policy.active_jobs.map((job) => <span className="policy-active-job" key={job.id}><span>{job.name}</span><code>{stateLabel(job.state)}</code></span>)}</div>}</div><div className="policy-gate-action">{!policy.queue_locked && <button className="button primary" data-policy-lock type="button" onClick={() => void actions.queueLock(true)}>{t("queue.lock")}</button>}<span className="policy-lock-state">{policy.queue_locked ? t("policy.queueLocked") : t("policy.queueUnlocked")}</span></div>
    </section>
    <div className="policy-layout"><PolicyCard title={t("policy.logCapacity")} description={t("policy.logDescription")} fields={LOG_FIELDS} policy={policy} draft={draft} canUpdate={policy.can_update} onDraft={(key, value) => setDraft((current) => ({ ...current, [key]: value }))} onSave={update} onReset={reset} /><PolicyCard title={t("policy.runtime")} description={t("policy.runtimeDescription")} fields={RUNTIME_FIELDS} policy={policy} draft={draft} canUpdate={policy.can_update} onDraft={(key, value) => setDraft((current) => ({ ...current, [key]: value }))} onSave={update} onReset={reset} /></div>
  </>;
}

function PolicyCard({ title, description, fields, policy, draft, canUpdate, onDraft, onSave, onReset }: { title: string; description: string; fields: FieldDefinition[]; policy: PolicyResponse; draft: Record<string, string>; canUpdate: boolean; onDraft: (key: string, value: string) => void; onSave: (field: FieldDefinition) => Promise<void>; onReset: (field: FieldDefinition) => Promise<void> }) {
  return <section className="panel policy-card"><div className="panel-header"><div className="panel-title"><div><h2>{title}</h2><p>{description}</p></div></div></div><div className="policy-fields">{fields.map((field) => <PolicyField field={field} policy={policy} value={draft[field.key] ?? ""} canUpdate={canUpdate} onDraft={onDraft} onSave={onSave} onReset={onReset} key={field.key} />)}</div></section>;
}

function PolicyField({ field, policy, value, canUpdate, onDraft, onSave, onReset }: { field: FieldDefinition; policy: PolicyResponse; value: string; canUpdate: boolean; onDraft: (key: string, value: string) => void; onSave: (field: FieldDefinition) => Promise<void>; onReset: (field: FieldDefinition) => Promise<void> }) {
  const { t, locale } = useI18n();
  const unit = field.unit === "MB" ? "MB" : t(field.unit === "jobs" ? "unit.jobs" : "unit.milliseconds");
  const current = readValue(policy, field);
  const defaultValue = readDefault(policy, field);
  const allowZero = field.key === "retention_jobs";
  const valid = policyInputIsValid(value, allowZero);
  return <form className="policy-field" onSubmit={(event) => { event.preventDefault(); if (canUpdate && valid) void onSave(field); }}><div className="policy-field-copy"><label htmlFor={`policy-${field.key}`}>{t(field.label)}</label><p>{t(field.description)}</p><small>{t("policy.currentDefault", { current: formatPolicyValue(current, unit, locale, t("policy.disabled")), default: formatPolicyValue(defaultValue, unit, locale, t("policy.disabled")) })}</small></div><div className="policy-field-control"><div className="policy-input-wrap"><input className="text-input" id={`policy-${field.key}`} data-policy-input={policyKey(field)} type="number" min={allowZero ? "0" : "1"} step="1" value={value} placeholder={field.key === "max_runtime_ms" ? t("policy.disabled") : undefined} disabled={!canUpdate} aria-invalid={Boolean(value) && !valid} onChange={(event) => onDraft(field.key, event.target.value)} /><span>{unit}</span></div>{value && !valid && <div className="form-feedback invalid">{allowZero ? t("policy.invalidZero") : t("policy.invalidPositive")}</div>}<div className="policy-field-actions"><button className="button small primary" data-policy-set={policyKey(field)} type="submit" disabled={!canUpdate || !valid}>{t("common.save")}</button><button className="button small secondary" data-policy-unset={policyKey(field)} type="button" disabled={!canUpdate} onClick={() => void onReset(field)}>{t("common.reset")}</button></div></div></form>;
}

function policyKey(field: FieldDefinition): string {
  const key = policyRouteKey(field.key);
  return field.section === "log" ? `log-${key}` : key;
}

function readValue(policy: PolicyResponse, field: FieldDefinition): number | null {
  return policy[field.section][field.key as keyof PolicyResponse[typeof field.section]] as number | null;
}

function readDefault(policy: PolicyResponse, field: FieldDefinition): number | null {
  return policy.defaults[field.section][field.key as keyof PolicyResponse[typeof field.section]] as number | null;
}
