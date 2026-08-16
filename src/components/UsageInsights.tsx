import type { ReactNode } from "react";
import {
  Activity,
  BarChart3,
  CheckCircle2,
  CircleAlert,
  Gauge,
  Layers,
  Sparkles,
  Timer,
  TrendingUp,
  Trophy,
  Zap,
} from "lucide-react";
import type { DailyStats, ModelStats, ProviderStats, UsageSummary } from "../types/usage";
import { fmtInt, fmtLatency, fmtPercent, fmtTokens, fmtUsd } from "../lib/format";
import { useI18n, type TranslationKey } from "../lib/i18n";
import { cn } from "../lib/utils";

interface Props {
  summary: UsageSummary | undefined;
  trends: DailyStats[] | undefined;
  providers: ProviderStats[] | undefined;
  models: ModelStats[] | undefined;
  isLoading: boolean;
}

type Tone = "cyan" | "violet" | "emerald" | "amber";

const TONE_CLASS: Record<Tone, string> = {
  cyan: "insight-card--cyan",
  violet: "insight-card--violet",
  emerald: "insight-card--emerald",
  amber: "insight-card--amber",
};

export function UsageInsights({
  summary,
  trends,
  providers,
  models,
  isLoading,
}: Props) {
  const { t } = useI18n();
  const totalCost = toNumber(summary?.totalCost);
  const totalRequests = summary?.totalRequests ?? 0;
  const averageCost = totalRequests > 0 ? totalCost / totalRequests : 0;
  const cacheHitRate = summary?.cacheHitRate ?? 0;
  const successRate = summary?.successRate ?? 0;
  const trendDelta = getTrendDelta(trends);
  const rankedModels = [...(models ?? [])]
    .sort((left, right) => toNumber(right.totalCost) - toNumber(left.totalCost))
    .slice(0, 3);
  const rankedProviders = [...(providers ?? [])]
    .sort((left, right) => right.requestCount - left.requestCount)
    .slice(0, 3);
  const maxModelCost = Math.max(...rankedModels.map((row) => toNumber(row.totalCost)), 1);
  const tokenParts = getTokenParts(summary, t);
  const tokenTotal = tokenParts.reduce((total, part) => total + part.value, 0);
  const insights = buildInsights({
    cacheHitRate,
    successRate,
    totalCost,
    totalRequests,
    trendDelta,
    topModel: rankedModels[0],
    topProvider: rankedProviders[0],
    realTotalTokens: summary?.realTotalTokens ?? 0,
    t,
  });

  return (
    <section className="insight-board">
      <header className="insight-board-header">
        <div>
          <div className="panel-kicker">{t("insights.kicker")}</div>
          <h2 className="panel-title">{t("insights.title")}</h2>
          <p className="insight-board-subtitle">
            {t("insights.subtitle")}
          </p>
        </div>
        <div className="insight-board-badge">
          <Sparkles className="h-3.5 w-3.5" />
          {t("insights.autoDerived")}
        </div>
      </header>

      <div className="insight-card-grid">
        <InsightCard
          icon={<Gauge className="h-4 w-4" />}
          label={t("insights.costEfficiency")}
          value={summary ? fmtUsd(averageCost) : "-"}
          note={t("insights.avgRequestCost")}
          tone="cyan"
          isLoading={isLoading}
        />
        <InsightCard
          icon={<Layers className="h-4 w-4" />}
          label={t("insights.cacheBenefit")}
          value={summary ? fmtPercent(cacheHitRate) : "-"}
          note={summary ? t("insights.reusedTokens", { tokens: fmtTokens(summary.cacheReadTokens) }) : t("common.waiting")}
          tone="violet"
          progress={cacheHitRate}
          isLoading={isLoading}
        />
        <InsightCard
          icon={<CheckCircle2 className="h-4 w-4" />}
          label={t("insights.stability")}
          value={summary ? fmtPercent(successRate) : "-"}
          note={t("insights.successRate")}
          tone="emerald"
          progress={successRate}
          isLoading={isLoading}
        />
        <InsightCard
          icon={<TrendingUp className="h-4 w-4" />}
          label={t("insights.momentum")}
          value={trendDelta === null ? "—" : formatSignedPercent(trendDelta)}
          note={t("insights.recentCostChange")}
          tone={trendDelta !== null && trendDelta > 0 ? "amber" : "cyan"}
          isLoading={isLoading}
        />
      </div>

      <div className="insight-lower-grid">
        <InsightPanel
          eyebrow="MODEL MIX"
          title={t("insights.modelContribution")}
          icon={<Trophy className="h-4 w-4" />}
        >
          {rankedModels.length === 0 ? (
            <EmptyInsightState isLoading={isLoading} label={t("insights.modelRankEmpty")} />
          ) : (
            <div className="leaderboard-list">
              {rankedModels.map((row, index) => {
                const rowCost = toNumber(row.totalCost);
                const share = totalCost > 0 ? (rowCost / totalCost) * 100 : 0;
                return (
                  <div className="leaderboard-row" key={row.model}>
                    <span
                      className={cn(
                        "leaderboard-index",
                        "leaderboard-index--" + (index + 1),
                      )}
                    >
                      {String(index + 1).padStart(2, "0")}
                    </span>
                    <div className="leaderboard-content">
                      <div className="leaderboard-line">
                        <span className="leaderboard-name" title={row.model}>
                          {row.model}
                        </span>
                        <span className="leaderboard-value">{fmtUsd(row.totalCost)}</span>
                      </div>
                      <div className="leaderboard-track">
                        <span
                          style={{
                            width: String(Math.min(100, (rowCost / maxModelCost) * 100)) + "%",
                          }}
                        />
                      </div>
                      <div className="leaderboard-meta">
                        <span>{fmtInt(row.requestCount)} requests</span>
                        <span>{t("insights.costShare", { share: fmtPercent(share) })}</span>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </InsightPanel>

        <InsightPanel
          eyebrow="PROVIDER HEALTH"
          title={t("insights.providerHealth")}
          icon={<Activity className="h-4 w-4" />}
        >
          {rankedProviders.length === 0 ? (
            <EmptyInsightState isLoading={isLoading} label={t("insights.providerHealthEmpty")} />
          ) : (
            <div className="provider-health-list">
              {rankedProviders.map((row) => {
                const health = getHealth(row.successRate, t);
                return (
                  <div
                    className="provider-health-row"
                    key={row.providerId + "-" + row.providerName}
                  >
                    <span className={cn("health-dot", health.dotClass)} />
                    <div className="provider-health-main">
                      <div className="provider-health-line">
                        <span className="provider-health-name" title={row.providerName}>
                          {row.providerName}
                        </span>
                        <span className={cn("health-label", health.labelClass)}>{health.label}</span>
                      </div>
                      <div className="provider-health-meta">
                        <span>{fmtPercent(row.successRate)} {t("insights.success")}</span>
                        <span className="provider-latency">
                          <Timer className="h-3 w-3" />
                          {row.avgLatencyMs === null
                            ? t("common.notAvailable")
                            : fmtLatency(row.avgLatencyMs)}
                        </span>
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </InsightPanel>

        <InsightPanel
          eyebrow="TOKEN COMPOSITION"
          title={t("insights.resourceComposition")}
          icon={<Layers className="h-4 w-4" />}
        >
          <TokenComposition parts={tokenParts} total={tokenTotal} isLoading={isLoading} t={t} />
        </InsightPanel>
      </div>

      <div className="insight-secondary-grid">
        <InsightPanel
          eyebrow="ACTIVITY RHYTHM"
          title={t("insights.activityRhythm")}
          icon={<BarChart3 className="h-4 w-4" />}
        >
          <ActivityRhythm trends={trends} isLoading={isLoading} t={t} />
        </InsightPanel>
        <InsightPanel
          eyebrow="SIGNAL FEED"
          title={t("insights.keyTakeaways")}
          icon={<Zap className="h-4 w-4" />}
        >
          <div className="insight-feed">
            {insights.map((insight, index) => (
              <div
                className={cn("insight-feed-item", "insight-feed-item--" + insight.tone)}
                key={index}
              >
                <span className="insight-feed-icon">{insight.icon}</span>
                <p>{insight.content}</p>
              </div>
            ))}
          </div>
        </InsightPanel>
      </div>
    </section>
  );
}

function InsightCard({
  icon,
  label,
  value,
  note,
  tone,
  progress,
  isLoading,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  note: string;
  tone: Tone;
  progress?: number;
  isLoading: boolean;
}) {
  return (
    <article className={cn("insight-card", TONE_CLASS[tone])}>
      <div className="insight-card-heading">
        <span className="insight-card-icon">{icon}</span>
        <span className="insight-card-label">{label}</span>
      </div>
      <div className="insight-card-value tabular-nums">{isLoading ? "..." : value}</div>
      <div className="insight-card-note">{note}</div>
      {progress !== undefined && (
        <div className="insight-card-progress">
          <span
            style={{
              width: String(Math.max(0, Math.min(100, progress))) + "%",
            }}
          />
        </div>
      )}
    </article>
  );
}

function InsightPanel({
  eyebrow,
  title,
  icon,
  children,
}: {
  eyebrow: string;
  title: string;
  icon: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="insight-subpanel">
      <header className="insight-subpanel-header">
        <div>
          <div className="insight-subpanel-eyebrow">{eyebrow}</div>
          <h3 className="insight-subpanel-title">{title}</h3>
        </div>
        <span className="insight-subpanel-icon">{icon}</span>
      </header>
      <div className="insight-subpanel-content">{children}</div>
    </section>
  );
}

function EmptyInsightState({ isLoading, label }: { isLoading: boolean; label: string }) {
  const { t } = useI18n();
  return <div className="insight-empty">{isLoading ? t("insights.analyzing") : label}</div>;
}

type TokenPart = {
  label: string;
  value: number;
  color: string;
};

function getTokenParts(
  summary: UsageSummary | undefined,
  t: (key: TranslationKey, values?: Record<string, string | number>) => string,
): TokenPart[] {
  return [
    { label: t("insights.tokenInput"), value: summary?.inputTokens ?? 0, color: "hsl(var(--primary))" },
    { label: t("insights.tokenOutput"), value: summary?.outputTokens ?? 0, color: "hsl(var(--accent))" },
    { label: t("insights.tokenCacheRead"), value: summary?.cacheReadTokens ?? 0, color: "hsl(var(--success))" },
    {
      label: t("insights.tokenCacheWrite"),
      value: summary?.cacheCreationTokens ?? 0,
      color: "hsl(var(--warning))",
    },
  ];
}

function TokenComposition({
  parts,
  total,
  isLoading,
  t,
}: {
  parts: TokenPart[];
  total: number;
  isLoading: boolean;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
}) {
  if (isLoading || total <= 0) {
    return <EmptyInsightState isLoading={isLoading} label={t("insights.tokenDataEmpty")} />;
  }

  return (
    <div>
      <div className="composition-layout">
        <div
          className="composition-ring"
          style={{ background: buildCompositionGradient(parts, total) }}
          aria-label={t("insights.tokenCompositionAria", { total: fmtTokens(total) })}
        >
          <div className="composition-ring-core">
            <strong className="composition-total">{fmtTokens(total)}</strong>
            <span className="composition-caption">{t("insights.totalTokens")}</span>
          </div>
        </div>
        <div className="composition-legend">
          {parts.map((part) => (
            <div className="composition-legend-item" key={part.label}>
              <span className="composition-legend-dot" style={{ backgroundColor: part.color }} />
              <span className="composition-legend-label">{part.label}</span>
              <span className="composition-legend-value">
                {fmtTokens(part.value)} · {fmtPercent((part.value / total) * 100)}
              </span>
            </div>
          ))}
        </div>
      </div>
      <div className="composition-stack" aria-hidden="true">
        {parts.map((part) => {
          const width = (part.value / total) * 100;
          return (
            <span
              className="composition-segment"
              key={part.label}
              style={{ width: String(width) + "%", backgroundColor: part.color }}
            />
          );
        })}
      </div>
    </div>
  );
}

function buildCompositionGradient(parts: TokenPart[], total: number): string {
  let cursor = 0;
  const stops = parts.map((part) => {
    const start = (cursor / total) * 100;
    cursor += part.value;
    const end = (cursor / total) * 100;
    return part.color + " " + start + "% " + end + "%";
  });
  return "conic-gradient(" + stops.join(", ") + ")";
}

function ActivityRhythm({
  trends,
  isLoading,
  t,
}: {
  trends: DailyStats[] | undefined;
  isLoading: boolean;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
}) {
  const rows = (trends ?? [])
    .filter((row) => row.requestCount > 0 || toNumber(row.totalCost) > 0)
    .slice(-12);
  if (rows.length === 0) {
    return <EmptyInsightState isLoading={isLoading} label={t("insights.activityEmpty")} />;
  }

  const maxRequests = Math.max(...rows.map((row) => row.requestCount), 1);
  const peak = rows.reduce(
    (best, row) => (row.requestCount > best.requestCount ? row : best),
    rows[0],
  );

  return (
    <div className="activity-rhythm">
      <div className="activity-bars" aria-label={t("insights.activityAria")}>
        {rows.map((row) => (
          <div
            className="activity-bar-column"
            key={row.date}
            title={formatActivityTitle(row, t)}
          >
            <span className="activity-bar-value">{fmtInt(row.requestCount)}</span>
            <div className="activity-bar-track">
              <span
                style={{
                  height: String(Math.max(8, (row.requestCount / maxRequests) * 100)) + "%",
                }}
              />
            </div>
            <span className="activity-bar-label">{formatActivityLabel(row.date)}</span>
          </div>
        ))}
      </div>
      <div className="activity-summary">
        <span>
          {t("insights.peakRequests", { count: fmtInt(peak.requestCount) })}
        </span>
        <span>{formatActivityLabel(peak.date)}</span>
      </div>
    </div>
  );
}

function formatActivityLabel(value: string): string {
  const day = value.slice(5, 10);
  return value.length > 10 ? value.slice(11, 16) || day : day;
}

function formatActivityTitle(
  row: DailyStats,
  t: (key: TranslationKey, values?: Record<string, string | number>) => string,
): string {
  return (
    formatActivityLabel(row.date) +
    " · " +
    fmtInt(row.requestCount) +
    " " +
    t("common.requests") +
    " · " +
    fmtUsd(row.totalCost)
  );
}

function toNumber(value: string | number | undefined): number {
  const parsed = typeof value === "number" ? value : Number.parseFloat(value ?? "");
  return Number.isFinite(parsed) ? parsed : 0;
}

function getTrendDelta(trends: DailyStats[] | undefined): number | null {
  const active = (trends ?? []).filter(
    (row) => row.requestCount > 0 || toNumber(row.totalCost) > 0,
  );
  if (active.length < 2) return null;
  const previous = toNumber(active[active.length - 2]?.totalCost);
  const latest = toNumber(active[active.length - 1]?.totalCost);
  if (previous <= 0) return null;
  return ((latest - previous) / previous) * 100;
}

function formatSignedPercent(value: number): string {
  const sign = value > 0 ? "+" : "";
  return sign + value.toFixed(1) + "%";
}

function getHealth(
  successRate: number,
  t: (key: TranslationKey, values?: Record<string, string | number>) => string,
): {
  label: string;
  labelClass: string;
  dotClass: string;
} {
  if (successRate >= 99) {
    return { label: t("insights.healthGood"), labelClass: "is-good", dotClass: "is-good" };
  }
  if (successRate >= 95) {
    return { label: t("insights.healthStable"), labelClass: "is-stable", dotClass: "is-stable" };
  }
  return { label: t("insights.healthWatch"), labelClass: "is-watch", dotClass: "is-watch" };
}

function buildInsights({
  cacheHitRate,
  successRate,
  totalCost,
  totalRequests,
  trendDelta,
  topModel,
  topProvider,
  realTotalTokens,
  t,
}: {
  cacheHitRate: number;
  successRate: number;
  totalCost: number;
  totalRequests: number;
  trendDelta: number | null;
  topModel: ModelStats | undefined;
  topProvider: ProviderStats | undefined;
  realTotalTokens: number;
  t: (key: TranslationKey, values?: Record<string, string | number>) => string;
}): Array<{ icon: ReactNode; tone: Tone; content: ReactNode }> {
  const insights: Array<{ icon: ReactNode; tone: Tone; content: ReactNode }> = [];

  if (topModel && totalCost > 0) {
    const share = (toNumber(topModel.totalCost) / totalCost) * 100;
    insights.push({
      icon: <Trophy className="h-3.5 w-3.5" />,
      tone: "violet",
      content: <>{t("insights.topModelConclusion", { model: topModel.model, share: fmtPercent(share) })}</>,
    });
  }

  if (cacheHitRate >= 20 && realTotalTokens > 0) {
    insights.push({
      icon: <Layers className="h-3.5 w-3.5" />,
      tone: "emerald",
      content: <>{t("insights.cacheGood", { rate: fmtPercent(cacheHitRate) })}</>,
    });
  } else if (realTotalTokens > 0) {
    insights.push({
      icon: <CircleAlert className="h-3.5 w-3.5" />,
      tone: "amber",
      content: <>{t("insights.cacheWatch", { rate: fmtPercent(cacheHitRate) })}</>,
    });
  }

  if (topProvider) {
    insights.push({
      icon: <Activity className="h-3.5 w-3.5" />,
      tone: successRate >= 95 ? "cyan" : "amber",
      content: <>{t("insights.topProviderConclusion", {
        provider: topProvider.providerName,
        count: fmtInt(topProvider.requestCount),
        rate: fmtPercent(topProvider.successRate),
      })}</>,
    });
  }

  if (trendDelta !== null) {
    insights.push({
      icon: <TrendingUp className="h-3.5 w-3.5" />,
      tone: trendDelta > 0 ? "amber" : "cyan",
      content: <>{t("insights.trendConclusion", {
        direction: trendDelta > 0 ? t("insights.trendUp") : t("insights.trendDown"),
        rate: fmtPercent(Math.abs(trendDelta)),
      })}</>,
    });
  }

  if (insights.length === 0 && totalRequests === 0) {
    insights.push({
      icon: <Sparkles className="h-3.5 w-3.5" />,
      tone: "cyan",
      content: <>{t("insights.emptyConclusion")}</>,
    });
  }

  return insights.slice(0, 4);
}
