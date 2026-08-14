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

export function UsageInsights({ summary, trends, providers, models, isLoading }: Props) {
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
  const tokenParts = getTokenParts(summary);
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
  });

  return (
    <section className="insight-board">
      <header className="insight-board-header">
        <div>
          <div className="panel-kicker">INTELLIGENCE LAYER</div>
          <h2 className="panel-title">数据洞察</h2>
          <p className="insight-board-subtitle">
            从当前筛选结果自动提炼效率、稳定性和资源集中度
          </p>
        </div>
        <div className="insight-board-badge">
          <Sparkles className="h-3.5 w-3.5" />
          AUTO-DERIVED
        </div>
      </header>

      <div className="insight-card-grid">
        <InsightCard
          icon={<Gauge className="h-4 w-4" />}
          label="成本效率"
          value={summary ? fmtUsd(averageCost) : "-"}
          note="平均每次请求成本"
          tone="cyan"
          isLoading={isLoading}
        />
        <InsightCard
          icon={<Layers className="h-4 w-4" />}
          label="缓存收益"
          value={summary ? fmtPercent(cacheHitRate) : "-"}
          note={summary ? fmtTokens(summary.cacheReadTokens) + " tokens 被复用" : "等待数据"}
          tone="violet"
          progress={cacheHitRate}
          isLoading={isLoading}
        />
        <InsightCard
          icon={<CheckCircle2 className="h-4 w-4" />}
          label="稳定性"
          value={summary ? fmtPercent(successRate) : "-"}
          note="请求成功率"
          tone="emerald"
          progress={successRate}
          isLoading={isLoading}
        />
        <InsightCard
          icon={<TrendingUp className="h-4 w-4" />}
          label="趋势动能"
          value={trendDelta === null ? "—" : formatSignedPercent(trendDelta)}
          note="最近两个活跃时段成本变化"
          tone={trendDelta !== null && trendDelta > 0 ? "amber" : "cyan"}
          isLoading={isLoading}
        />
      </div>

      <div className="insight-lower-grid">
        <InsightPanel
          eyebrow="MODEL MIX"
          title="模型贡献度"
          icon={<Trophy className="h-4 w-4" />}
        >
          {rankedModels.length === 0 ? (
            <EmptyInsightState isLoading={isLoading} label="模型数据积累后显示贡献排行" />
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
                        <span>{fmtPercent(share)} 成本占比</span>
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
          title="供应商体检"
          icon={<Activity className="h-4 w-4" />}
        >
          {rankedProviders.length === 0 ? (
            <EmptyInsightState isLoading={isLoading} label="供应商数据积累后显示健康度" />
          ) : (
            <div className="provider-health-list">
              {rankedProviders.map((row) => {
                const health = getHealth(row.successRate);
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
                        <span>{fmtPercent(row.successRate)} 成功</span>
                        <span className="provider-latency">
                          <Timer className="h-3 w-3" />
                          {fmtLatency(row.avgLatencyMs)}
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
          title="资源构成"
          icon={<Layers className="h-4 w-4" />}
        >
          <TokenComposition parts={tokenParts} total={tokenTotal} isLoading={isLoading} />
        </InsightPanel>
      </div>

      <div className="insight-secondary-grid">
        <InsightPanel
          eyebrow="ACTIVITY RHYTHM"
          title="活跃节奏"
          icon={<BarChart3 className="h-4 w-4" />}
        >
          <ActivityRhythm trends={trends} isLoading={isLoading} />
        </InsightPanel>
        <InsightPanel
          eyebrow="SIGNAL FEED"
          title="关键结论"
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
  return <div className="insight-empty">{isLoading ? "正在分析..." : label}</div>;
}

type TokenPart = {
  label: string;
  value: number;
  color: string;
};

function getTokenParts(summary: UsageSummary | undefined): TokenPart[] {
  return [
    { label: "输入", value: summary?.inputTokens ?? 0, color: "hsl(var(--primary))" },
    { label: "输出", value: summary?.outputTokens ?? 0, color: "hsl(var(--accent))" },
    { label: "缓存读", value: summary?.cacheReadTokens ?? 0, color: "hsl(var(--success))" },
    {
      label: "缓存写",
      value: summary?.cacheCreationTokens ?? 0,
      color: "hsl(var(--warning))",
    },
  ];
}

function TokenComposition({
  parts,
  total,
  isLoading,
}: {
  parts: TokenPart[];
  total: number;
  isLoading: boolean;
}) {
  if (isLoading || total <= 0) {
    return <EmptyInsightState isLoading={isLoading} label="有 Token 数据后显示资源构成" />;
  }

  return (
    <div>
      <div className="composition-layout">
        <div
          className="composition-ring"
          style={{ background: buildCompositionGradient(parts, total) }}
          aria-label={"Token 构成，总计 " + fmtTokens(total)}
        >
          <div className="composition-ring-core">
            <strong className="composition-total">{fmtTokens(total)}</strong>
            <span className="composition-caption">总 Tokens</span>
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
}: {
  trends: DailyStats[] | undefined;
  isLoading: boolean;
}) {
  const rows = (trends ?? [])
    .filter((row) => row.requestCount > 0 || toNumber(row.totalCost) > 0)
    .slice(-12);
  if (rows.length === 0) {
    return <EmptyInsightState isLoading={isLoading} label="有趋势数据后显示活跃节奏" />;
  }

  const maxRequests = Math.max(...rows.map((row) => row.requestCount), 1);
  const peak = rows.reduce(
    (best, row) => (row.requestCount > best.requestCount ? row : best),
    rows[0],
  );

  return (
    <div className="activity-rhythm">
      <div className="activity-bars" aria-label="请求活跃节奏">
        {rows.map((row) => (
          <div
            className="activity-bar-column"
            key={row.date}
            title={formatActivityTitle(row)}
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
          峰值 <strong>{fmtInt(peak.requestCount)}</strong> requests
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

function formatActivityTitle(row: DailyStats): string {
  return (
    formatActivityLabel(row.date) +
    " · " +
    fmtInt(row.requestCount) +
    " requests · " +
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

function getHealth(successRate: number): {
  label: string;
  labelClass: string;
  dotClass: string;
} {
  if (successRate >= 99) {
    return { label: "优秀", labelClass: "is-good", dotClass: "is-good" };
  }
  if (successRate >= 95) {
    return { label: "稳定", labelClass: "is-stable", dotClass: "is-stable" };
  }
  return { label: "观察", labelClass: "is-watch", dotClass: "is-watch" };
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
}: {
  cacheHitRate: number;
  successRate: number;
  totalCost: number;
  totalRequests: number;
  trendDelta: number | null;
  topModel: ModelStats | undefined;
  topProvider: ProviderStats | undefined;
  realTotalTokens: number;
}): Array<{ icon: ReactNode; tone: Tone; content: ReactNode }> {
  const insights: Array<{ icon: ReactNode; tone: Tone; content: ReactNode }> = [];

  if (topModel && totalCost > 0) {
    const share = (toNumber(topModel.totalCost) / totalCost) * 100;
    insights.push({
      icon: <Trophy className="h-3.5 w-3.5" />,
      tone: "violet",
      content: (
        <>
          <strong>{topModel.model}</strong> 是当前成本主力，占总成本 {fmtPercent(share)}。
        </>
      ),
    });
  }

  if (cacheHitRate >= 20 && realTotalTokens > 0) {
    insights.push({
      icon: <Layers className="h-3.5 w-3.5" />,
      tone: "emerald",
      content: <>缓存命中率已达 {fmtPercent(cacheHitRate)}，上下文复用正在降低重复消耗。</>,
    });
  } else if (realTotalTokens > 0) {
    insights.push({
      icon: <CircleAlert className="h-3.5 w-3.5" />,
      tone: "amber",
      content: <>缓存命中率为 {fmtPercent(cacheHitRate)}，可以关注重复上下文是否偏多。</>,
    });
  }

  if (topProvider) {
    insights.push({
      icon: <Activity className="h-3.5 w-3.5" />,
      tone: successRate >= 95 ? "cyan" : "amber",
      content: (
        <>
          <strong>{topProvider.providerName}</strong> 承担 {fmtInt(topProvider.requestCount)} 次请求，成功率{" "}
          {fmtPercent(topProvider.successRate)}。
        </>
      ),
    });
  }

  if (trendDelta !== null) {
    insights.push({
      icon: <TrendingUp className="h-3.5 w-3.5" />,
      tone: trendDelta > 0 ? "amber" : "cyan",
      content: (
        <>
          最近一个活跃时段成本 {trendDelta > 0 ? "上升" : "下降"} {fmtPercent(Math.abs(trendDelta))}。
        </>
      ),
    });
  }

  if (insights.length === 0 && totalRequests === 0) {
    insights.push({
      icon: <Sparkles className="h-3.5 w-3.5" />,
      tone: "cyan",
      content: <>同步完成并积累数据后，这里会自动生成分析结论。</>,
    });
  }

  return insights.slice(0, 4);
}
