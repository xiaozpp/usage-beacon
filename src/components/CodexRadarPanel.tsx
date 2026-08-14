import type { CSSProperties } from "react";
import {
  AlertTriangle,
  BrainCircuit,
  CircleDollarSign,
  Radio,
  RefreshCw,
} from "lucide-react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { useCodexRadar } from "../lib/hooks";
import { fmtInt, fmtUsd } from "../lib/format";
import type { RadarIqPoint, RadarQuotaHistoryPoint } from "../types/usage";

const EFFORTS = ["low", "medium", "high", "xhigh", "max", "ultra"] as const;
const MODEL_ORDER = ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5"];
const MODEL_LABELS: Record<string, string> = {
  "gpt-5.6-sol": "5.6 Sol",
  "gpt-5.6-terra": "5.6 Terra",
  "gpt-5.6-luna": "5.6 Luna",
  "gpt-5.5": "GPT-5.5",
};
const EFFORT_LABELS: Record<string, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra High",
  max: "Max",
  ultra: "Ultra",
};

export function CodexRadarPanel() {
  const radar = useCodexRadar();
  const iq = radar.data?.iq;
  const quota = radar.data?.quota;
  const reliablePoints = (iq?.points ?? []).filter(
    (point) => point.model.startsWith("gpt-") && point.total >= 30,
  );
  const models = [...new Set(reliablePoints.map((point) => point.model))].sort(
    (left, right) => modelRank(left) - modelRank(right),
  );
  const pointMap = new Map(
    reliablePoints.map((point) => [`${point.model}:${point.effort}`, point]),
  );
  const strongest = [...reliablePoints].sort((left, right) => right.iq - left.iq)[0];
  const radarPricePick = [...reliablePoints]
    .filter((point) => point.iq >= 90 && point.averagePriceUsd !== null)
    .sort((left, right) => (left.averagePriceUsd ?? Infinity) - (right.averagePriceUsd ?? Infinity))[0];
  const tiers = [...(quota?.tiers ?? [])].sort((left, right) => {
    return quotaRank(left.plan) - quotaRank(right.plan);
  });
  const history = quota?.history ?? [];
  const historyChange = getHistoryChange(history);

  return (
    <section className="radar-board">
      <header className="radar-board-header">
        <div className="radar-board-title-group">
          <div className="radar-orb" aria-hidden="true">
            <BrainCircuit className="h-5 w-5" />
          </div>
          <div>
            <div className="panel-kicker">CODEX RADAR / NETWORK</div>
            <h2 className="panel-title">联网智商与额度雷达</h2>
            <p className="radar-board-subtitle">社区实测能力、任务均价与订阅周容量</p>
          </div>
        </div>
        <div className="radar-board-actions">
          <span className={`radar-live-badge ${radar.isError ? "is-error" : ""}`}>
            <Radio className="h-3.5 w-3.5" />
            {radar.isError ? "网络不可用" : radar.isFetching ? "联网更新中" : "社区数据在线"}
          </span>
          <button
            type="button"
            className="radar-refresh-button"
            onClick={() => void radar.refetch()}
            disabled={radar.isFetching}
            title="重新获取 Codex Radar 数据"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${radar.isFetching ? "animate-spin" : ""}`} />
            刷新
          </button>
        </div>
      </header>

      {radar.isLoading ? (
        <div className="radar-loading" role="status">
          正在连接 Codex Radar…
        </div>
      ) : radar.isError && !radar.data ? (
        <div className="radar-error" role="alert">
          <AlertTriangle className="h-4 w-4" />
          <div>
            <strong>雷达暂时无法联网</strong>
            <span>本地使用统计不受影响，可以稍后重试。</span>
          </div>
        </div>
      ) : (
        <>
          {radar.data?.warnings.length ? (
            <div className="radar-warning" role="status">
              <AlertTriangle className="h-3.5 w-3.5" />
              {radar.data.warnings.join("；")}
            </div>
          ) : null}

          <div className="radar-kpi-strip">
            <RadarKpi
              label="当前最高 IQ"
              value={strongest ? strongest.iq.toFixed(1) : "—"}
              note={strongest ? `${modelLabel(strongest.model)} · ${effortLabel(strongest.effort)}` : "等待数据"}
            />
            <RadarKpi
              label="近 24h 众测"
              value={iq ? fmtInt(iq.runs24hTotal) : "—"}
              note={iq ? `累计 ${fmtInt(iq.runsTotal)} 次` : "等待数据"}
            />
            <RadarKpi
              label="90+ IQ 雷达均价最低"
              value={radarPricePick?.averagePriceUsd != null ? fmtUsd(radarPricePick.averagePriceUsd) : "—"}
              note={radarPricePick ? `${modelLabel(radarPricePick.model)} · ${effortLabel(radarPricePick.effort)}` : "等待数据"}
              price
            />
          </div>

          <div className="radar-content-grid">
            <article className="radar-panel radar-iq-panel">
              <div className="radar-panel-head">
                <div>
                  <div className="panel-kicker">IQ HEATMAP</div>
                  <h3>模型 × 推理强度</h3>
                  <p>仅展示至少 30 个任务样本的 Codex 配置，单元格为 IQ</p>
                </div>
                <span>{formatSourceTime(iq?.sourceUpdatedAt)}</span>
              </div>
              {models.length ? (
                <div className="radar-heatmap-scroll">
                  <div className="radar-heatmap" role="table" aria-label="Codex 模型智商热力矩阵">
                    <div className="radar-heatmap-row is-head" role="row">
                      <span role="columnheader">模型</span>
                      {EFFORTS.map((effort) => (
                        <span role="columnheader" key={effort}>{effortLabel(effort)}</span>
                      ))}
                    </div>
                    {models.map((model) => (
                      <div className="radar-heatmap-row" role="row" key={model}>
                        <strong className="radar-model-label" role="rowheader">{modelLabel(model)}</strong>
                        {EFFORTS.map((effort) => (
                          <IqCell point={pointMap.get(`${model}:${effort}`)} key={effort} />
                        ))}
                      </div>
                    ))}
                  </div>
                </div>
              ) : (
                <div className="radar-empty">暂无可靠的 Codex 智商样本</div>
              )}
            </article>

            <article className="radar-panel radar-quota-panel">
              <div className="radar-panel-head">
                <div>
                  <div className="panel-kicker">QUOTA RADAR</div>
                  <h3>订阅 7 天容量</h3>
                  <p>站方任务测量与档位推算，不是个人账号实时余额</p>
                </div>
                <span>{quota?.sourceUpdatedAt || "等待数据"}</span>
              </div>
              {tiers.length ? (
                <div className="quota-tier-grid">
                  {tiers.map((tier) => (
                    <div className="quota-tier-card" key={tier.plan}>
                      <span>{tier.plan}</span>
                      <strong>{formatRadarUsd(tier.weeklyUsd)}</strong>
                      <small>{tier.source}</small>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="radar-empty">暂未取得额度档位</div>
              )}
              {history.length >= 4 ? (
                <div className="quota-trend-block">
                  <div className="quota-trend-head">
                    <div>
                      <strong>20x Pro 历史容量</strong>
                      <span>聚焦刻度，不从零起</span>
                    </div>
                    {historyChange ? (
                      <span className={historyChange.delta < 0 ? "is-down" : "is-up"}>
                        {formatRadarUsd(historyChange.start)} → {formatRadarUsd(historyChange.end)} · {formatSigned(historyChange.percent)}
                      </span>
                    ) : null}
                  </div>
                  <div className="quota-trend-chart">
                    <ResponsiveContainer width="100%" height="100%">
                      <LineChart data={history} margin={{ top: 8, right: 8, left: 0, bottom: 2 }}>
                        <CartesianGrid vertical={false} strokeDasharray="3 3" stroke="hsl(var(--border))" />
                        <XAxis
                          dataKey="label"
                          tickFormatter={compactHistoryLabel}
                          tickLine={false}
                          axisLine={false}
                          minTickGap={22}
                          fontSize={10}
                          stroke="hsl(var(--muted-foreground))"
                        />
                        <YAxis
                          domain={["auto", "auto"]}
                          tickFormatter={(value) => `$${Math.round(Number(value)).toLocaleString("en-US")}`}
                          tickLine={false}
                          axisLine={false}
                          width={56}
                          fontSize={10}
                          stroke="hsl(var(--muted-foreground))"
                        />
                        <Tooltip
                          formatter={(value) => [formatRadarUsd(Number(value)), "20x Pro 7d"]}
                          labelFormatter={(label) => String(label)}
                          contentStyle={{
                            background: "hsl(var(--card) / 0.98)",
                            border: "1px solid hsl(var(--border))",
                            borderRadius: "10px",
                            fontSize: "12px",
                          }}
                        />
                        <Line
                          type="monotone"
                          dataKey="weeklyUsd"
                          stroke="#2563eb"
                          strokeWidth={2.4}
                          dot={{ r: 2.8, fill: "#ffffff", stroke: "#2563eb", strokeWidth: 2 }}
                          activeDot={{ r: 4 }}
                          isAnimationActive={false}
                        />
                      </LineChart>
                    </ResponsiveContainer>
                  </div>
                </div>
              ) : null}
            </article>
          </div>

          <div className="radar-source-note">
            <CircleDollarSign className="h-3.5 w-3.5" />
            雷达价格优先显示为 DeepSWE 单次任务实测均价；IQ 为社区加权通过率 × 1.5。本地请求成本仍按日志口径独立统计。
          </div>
        </>
      )}
    </section>
  );
}

function RadarKpi({
  label,
  value,
  note,
  price = false,
}: {
  label: string;
  value: string;
  note: string;
  price?: boolean;
}) {
  return (
    <div className={`radar-kpi ${price ? "is-price" : ""}`}>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{note}</small>
    </div>
  );
}

function IqCell({ point }: { point: RadarIqPoint | undefined }) {
  if (!point) return <span className="radar-iq-cell is-empty" role="cell">—</span>;
  const level = Math.max(0.08, Math.min(1, point.iq / 115));
  const title = [
    `${modelLabel(point.model)} · ${effortLabel(point.effort)}`,
    `IQ ${point.iq.toFixed(1)}`,
    point.averagePriceUsd == null ? null : `雷达实测任务均价 ${fmtUsd(point.averagePriceUsd)}`,
    point.averageMinutes == null ? null : `平均 ${point.averageMinutes.toFixed(1)} 分钟`,
    `${point.total} 个任务样本`,
  ].filter(Boolean).join(" · ");
  return (
    <span
      className="radar-iq-cell"
      role="cell"
      title={title}
      style={{ "--iq-level": level } as CSSProperties}
    >
      {point.iq.toFixed(1)}
    </span>
  );
}

function modelRank(model: string) {
  const index = MODEL_ORDER.indexOf(model);
  return index === -1 ? MODEL_ORDER.length : index;
}

function quotaRank(plan: string) {
  if (plan === "Plus") return 0;
  if (plan.startsWith("5x")) return 1;
  if (plan.startsWith("20x")) return 2;
  return 3;
}

function modelLabel(model: string) {
  return MODEL_LABELS[model] ?? model;
}

function effortLabel(effort: string) {
  return EFFORT_LABELS[effort] ?? effort;
}

function formatSourceTime(value: string | undefined) {
  if (!value) return "等待数据";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return `${date.toLocaleDateString("zh-CN", { month: "numeric", day: "numeric" })} ${date.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })} 更新`;
}

function compactHistoryLabel(value: string) {
  const match = value.match(/(\d{4})-(\d{2})-(\d{2})/);
  return match ? `${Number(match[2])}/${Number(match[3])}` : value;
}

function getHistoryChange(history: RadarQuotaHistoryPoint[]) {
  const first = history[0];
  const last = history[history.length - 1];
  if (!first || !last || first.weeklyUsd <= 0) return null;
  const delta = last.weeklyUsd - first.weeklyUsd;
  return {
    start: first.weeklyUsd,
    end: last.weeklyUsd,
    delta,
    percent: (delta / first.weeklyUsd) * 100,
  };
}

function formatSigned(value: number) {
  return `${value >= 0 ? "+" : ""}${value.toFixed(1)}%`;
}

function formatRadarUsd(value: number) {
  if (value < 1_000) return fmtUsd(value);
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}
