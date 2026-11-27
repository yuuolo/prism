import type { ReactNode } from "react";
import { t } from "./i18n";

/** Native disclosure: keyboard accessible, closed by default, no hover needed. */
export default function Help({ title, children, compact = false }: { title: string; children: ReactNode; compact?: boolean }) {
  const label = `${title} · ${t("帮助")}`;
  return <details className="help-disclosure">
    <summary aria-label={label} title={label}>
      <span className="help-mark" aria-hidden="true">?</span>
      {!compact && <span>{t("帮助")}</span>}
    </summary>
    <div className="help-body">{children}</div>
  </details>;
}
