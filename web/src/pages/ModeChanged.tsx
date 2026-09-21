import { useWorkspace } from "../context";
import { useI18n } from "../i18n/context";

/** A deliberate data boundary after the server reports a different workspace mode. */
export function ModeChanged() {
  const { state, actions } = useWorkspace();
  const { t } = useI18n();
  const mode = state.modeTransition!.actualMode;
  return <section className="empty-state mode-changed"><div className="empty-icon" aria-hidden="true">⇄</div><h1>{t("modeChanged.title", { mode })}</h1><p>{t("modeChanged.description")}</p><div className="page-actions"><button className="button primary" type="button" onClick={() => void actions.acceptModeTransition()}>{t("modeChanged.switch")}</button><button className="button secondary" type="button" onClick={actions.deferModeTransition}>{t("modeChanged.later")}</button><button className="button secondary" type="button" onClick={() => void actions.loadData()}>{t("common.retry")}</button></div></section>;
}
