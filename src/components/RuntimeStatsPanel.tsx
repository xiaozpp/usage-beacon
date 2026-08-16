import {
  BrainCircuit,
  Gauge,
  ListChecks,
  MessagesSquare,
  Timer,
  Wrench,
} from "lucide-react";
import type { RuntimeStats } from "../types/usage";
import { fmtDuration, fmtInt } from "../lib/format";
import { useI18n } from "../lib/i18n";

interface Props {
  stats: RuntimeStats | undefined;
  isLoading: boolean;
}

export function RuntimeStatsPanel({ stats, isLoading }: Props) {
  const { t } = useI18n();
  const hasData = Boolean(stats && stats.sessionCount > 0);
  const metrics = [
    {
      icon: <MessagesSquare className="h-3.5 w-3.5 text-sky-600 dark:text-sky-400" />,
      label: t("runtime.turns"),
      value: stats ? fmtInt(stats.turns) : "-",
    },
    {
      icon: <ListChecks className="h-3.5 w-3.5 text-violet-600 dark:text-violet-400" />,
      label: t("runtime.steps"),
      value: stats ? fmtInt(stats.steps) : "-",
    },
    {
      icon: <BrainCircuit className="h-3.5 w-3.5 text-indigo-600 dark:text-indigo-400" />,
      label: t("runtime.llm"),
      value: stats ? fmtDuration(stats.llmMs) : "-",
    },
    {
      icon: <Wrench className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400" />,
      label: t("runtime.tools"),
      value: stats ? fmtDuration(stats.toolMs) : "-",
    },
    {
      icon: <Timer className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />,
      label: t("runtime.averageTtft"),
      value: stats?.averageTtftMs != null ? fmtDuration(stats.averageTtftMs) : "-",
    },
    {
      icon: <Gauge className="h-3.5 w-3.5 text-rose-600 dark:text-rose-400" />,
      label: t("runtime.decodeRate"),
      value:
        stats?.decodeTokensPerSecond != null
          ? `${stats.decodeTokensPerSecond.toFixed(0)} tok/s`
          : "-",
    },
  ];

  return (
    <section className="insight-subpanel">
      <header className="insight-subpanel-header">
        <div>
          <div className="insight-subpanel-eyebrow">{t("runtime.kicker")}</div>
          <h3 className="insight-subpanel-title">{t("runtime.title")}</h3>
        </div>
        <span className="insight-subpanel-icon" title={t("runtime.officialMetric")}>
          <Gauge className="h-4 w-4" />
        </span>
      </header>
      <div className="insight-subpanel-content">
        {!isLoading && !hasData ? (
          <div className="insight-empty h-full min-h-32">{t("runtime.empty")}</div>
        ) : (
          <>
            <div className="grid grid-cols-2 gap-x-4 gap-y-3">
              {metrics.map((metric) => (
                <div key={metric.label} className="min-w-0">
                  <div className="flex items-center gap-1.5 text-[0.66rem] text-muted-foreground">
                    {metric.icon}
                    <span className="truncate">{metric.label}</span>
                  </div>
                  <div className="mt-1 text-sm font-semibold tabular-nums text-foreground">
                    {isLoading ? "..." : metric.value}
                  </div>
                </div>
              ))}
            </div>
            <div className="mt-4 flex flex-wrap items-center gap-x-2 gap-y-1 text-[0.62rem] text-muted-foreground">
              <span>{t("runtime.sources", { count: fmtInt(stats?.sourceCount ?? 0) })}</span>
              <span>·</span>
              <span>{t("runtime.sessions", { count: fmtInt(stats?.sessionCount ?? 0) })}</span>
              <span>·</span>
              <span>{t("runtime.decodeTokens", { count: fmtInt(stats?.decodeTokens ?? 0) })}</span>
            </div>
          </>
        )}
      </div>
    </section>
  );
}
