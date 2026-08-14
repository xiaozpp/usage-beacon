import { useState } from "react";
import {
  Area,
  AreaChart,
  Bar,
  CartesianGrid,
  ComposedChart,
  Legend,
  Line,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { DailyStats } from "../types/usage";
import { fmtUsd, fmtTokens } from "../lib/format";

interface Props {
  data: DailyStats[] | undefined;
  isLoading: boolean;
  startDate: number;
  endDate: number;
  rangeLabel?: string;
}

const HOUR_SECONDS = 60 * 60;
const DAY_SECONDS = 24 * HOUR_SECONDS;
const HOURLY_RANGE_LIMIT = 2 * DAY_SECONDS;

type Granularity = "hour" | "day";

interface TrendPoint {
  bucket: string;
  label: string;
  cost: number;
  requests: number;
  total: number;
  input: number;
  output: number;
  cacheCreation: number;
  cacheRead: number;
  cumulative: number;
}

const SERIES = [
  { key: "cost", label: "成本", color: "#fb7185", dashed: true },
  { key: "cacheCreation", label: "缓存创建", color: "#fbbf24", dashed: false },
  { key: "cacheRead", label: "缓存命中", color: "#a78bfa", dashed: false },
  { key: "input", label: "输入", color: "#22d3ee", dashed: false },
  { key: "output", label: "输出", color: "#34d399", dashed: false },
] as const;

function getGranularity(startDate: number, endDate: number): Granularity {
  return endDate - startDate <= HOURLY_RANGE_LIMIT ? "hour" : "day";
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

function fmtAxisUsd(value: number): string {
  if (!Number.isFinite(value) || Math.abs(value) < 0.005) return "$0";
  if (Math.abs(value) < 1) return `$${value.toFixed(2)}`;
  return `$${value.toFixed(0)}`;
}

function getBucketKey(date: Date, granularity: Granularity): string {
  const day = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  return granularity === "hour" ? `${day} ${pad(date.getHours())}` : day;
}

function getSourceBucketKey(value: string, granularity: Granularity): string {
  return granularity === "hour" ? value.slice(0, 13) : value.slice(0, 10);
}

function formatBucketLabel(date: Date, granularity: Granularity): string {
  const day = `${pad(date.getMonth() + 1)}/${pad(date.getDate())}`;
  return granularity === "hour" ? `${day} ${pad(date.getHours())}:00` : day;
}

function emptyTrendPoint(bucket: string, label: string): TrendPoint {
  return {
    bucket,
    label,
    cost: 0,
    requests: 0,
    total: 0,
    input: 0,
    output: 0,
    cacheCreation: 0,
    cacheRead: 0,
    cumulative: 0,
  };
}

function toTrendPoint(stats: DailyStats, bucket: string, label: string): TrendPoint {
  return {
    bucket,
    label,
    cost: Number.parseFloat(stats.totalCost) || 0,
    requests: stats.requestCount,
    total: stats.totalTokens,
    input: stats.inputTokens,
    output: stats.outputTokens,
    cacheCreation: stats.cacheCreationTokens,
    cacheRead: stats.cacheReadTokens,
    cumulative: 0,
  };
}

function buildChartData(
  data: DailyStats[],
  startDate: number,
  endDate: number,
): { data: TrendPoint[]; granularity: Granularity } {
  const granularity = getGranularity(startDate, endDate);
  const source = new Map(
    data.map((stats) => [getSourceBucketKey(stats.date, granularity), stats]),
  );
  const shouldFillGaps = granularity === "hour" || endDate - startDate <= 90 * DAY_SECONDS;

  if (!shouldFillGaps) {
    let cumulative = 0;
    return {
      granularity,
      data: data.map((stats) => {
        const parts = stats.date.slice(0, 10).split("-").map(Number);
        const month = parts[1] ?? 0;
        const day = parts[2] ?? 0;
        const label = month && day ? `${pad(month)}/${pad(day)}` : stats.date;
        const point = toTrendPoint(stats, getSourceBucketKey(stats.date, granularity), label);
        cumulative += point.total;
        return { ...point, cumulative };
      }),
    };
  }

  const cursor = new Date(startDate * 1000);
  if (granularity === "hour") {
    cursor.setMinutes(0, 0, 0);
  } else {
    cursor.setHours(0, 0, 0, 0);
  }

  const end = endDate * 1000;
  const points: TrendPoint[] = [];
  let guard = 0;
  while (cursor.getTime() <= end && guard++ < 10_000) {
    const bucket = getBucketKey(cursor, granularity);
    const label = formatBucketLabel(cursor, granularity);
    const stats = source.get(bucket);
    points.push(stats ? toTrendPoint(stats, bucket, label) : emptyTrendPoint(bucket, label));
    if (granularity === "hour") {
      cursor.setHours(cursor.getHours() + 1);
    } else {
      cursor.setDate(cursor.getDate() + 1);
    }
  }

  let cumulative = 0;
  return {
    granularity,
    data: points.map((point) => {
      cumulative += point.total;
      return { ...point, cumulative };
    }),
  };
}

function TrendLegend() {
  return (
    <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 pt-2 text-xs">
      {SERIES.map((series) => (
        <span
          key={series.key}
          className="inline-flex items-center gap-1.5"
          style={{ color: series.color }}
        >
          <span
            className={`inline-block w-5 border-t-2 ${series.dashed ? "border-dashed" : ""}`}
            style={{ borderColor: series.color }}
          />
          {series.label}
        </span>
      ))}
    </div>
  );
}

function CumulativeLegend() {
  const items = [
    { label: "时段真实 Tokens", color: "#0ea5e9", kind: "bar" },
    { label: "缓存命中", color: "#8b5cf6", kind: "line" },
    { label: "累计真实 Tokens", color: "#f97316", kind: "line" },
  ];

  return (
    <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 pt-2 text-xs">
      {items.map((item) => (
        <span key={item.label} className="inline-flex items-center gap-1.5" style={{ color: item.color }}>
          <span
            className={item.kind === "bar" ? "inline-block h-2.5 w-3 rounded-sm" : "inline-block w-5 border-t-2"}
            style={item.kind === "bar" ? { backgroundColor: item.color } : { borderColor: item.color }}
          />
          {item.label}
        </span>
      ))}
    </div>
  );
}

export function UsageTrendChart({
  data,
  isLoading,
  startDate,
  endDate,
  rangeLabel,
}: Props) {
  const [view, setView] = useState<"detail" | "cumulative">("detail");
  const chart = data ? buildChartData(data, startDate, endDate) : null;
  const chartData = chart?.data ?? [];
  const granularity = chart?.granularity ?? getGranularity(startDate, endDate);
  const activePoints = chartData.filter(
    (point) => point.requests > 0 || point.total > 0 || point.cost > 0,
  );
  const totalTokens = chartData[chartData.length - 1]?.cumulative ?? 0;
  const totalRequests = activePoints.reduce((sum, point) => sum + point.requests, 0);
  const peak = activePoints.reduce<TrendPoint | undefined>(
    (best, point) => (!best || point.total > best.total ? point : best),
    undefined,
  );
  const activeUnit = granularity === "hour" ? "小时" : "天";
  const hasData = activePoints.length > 0;

  return (
    <div className="analytics-panel">
      <div className="panel-header flex items-center justify-between gap-3">
        <div>
          <div className="panel-kicker">TELEMETRY / TREND</div>
          <h2 className="panel-title">使用趋势与峰值</h2>
        </div>
        <div className="trend-toolbar">
          <div className="trend-mode-switcher" role="group" aria-label="趋势图视图">
            <button
              type="button"
              className={`trend-mode-button ${view === "detail" ? "is-active" : ""}`}
              aria-pressed={view === "detail"}
              onClick={() => setView("detail")}
            >
              消耗明细
            </button>
            <button
              type="button"
              className={`trend-mode-button ${view === "cumulative" ? "is-active" : ""}`}
              aria-pressed={view === "cumulative"}
              onClick={() => setView("cumulative")}
            >
              峰值累计
            </button>
          </div>
          <div className="panel-meta">
            {rangeLabel && <span>{rangeLabel}</span>}
            {isLoading && <span>加载中...</span>}
          </div>
        </div>
      </div>
      <div className="trend-summary-strip">
        <div className="trend-summary-item">
          <span className="trend-summary-label">区间累计</span>
          <strong className="trend-summary-value">{fmtTokens(totalTokens)}</strong>
          <span className="trend-summary-note">真实 Tokens</span>
        </div>
        <div className="trend-summary-item is-peak">
          <span className="trend-summary-label">真实 Token 峰值</span>
          <strong className="trend-summary-value">{peak ? fmtTokens(peak.total) : "—"}</strong>
          <span className="trend-summary-note">{peak?.label ?? "暂无活跃时段"}</span>
        </div>
        <div className="trend-summary-item">
          <span className="trend-summary-label">活跃{activeUnit}</span>
          <strong className="trend-summary-value">{activePoints.length}</strong>
          <span className="trend-summary-note">有请求或消耗</span>
        </div>
        <div className="trend-summary-item">
          <span className="trend-summary-label">请求总量</span>
          <strong className="trend-summary-value">{totalRequests.toLocaleString("zh-CN")}</strong>
          <span className="trend-summary-note">events</span>
        </div>
      </div>
      <div className="chart-stage h-[360px] w-auto md:h-[400px]">
        {!hasData ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {isLoading ? "加载中..." : "暂无数据"}
          </div>
        ) : view === "detail" ? (
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={chartData} margin={{ top: 8, right: 4, left: 2, bottom: 22 }}>
              <defs>
                <linearGradient id="cacheReadGradient" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%" stopColor="#a78bfa" stopOpacity={0.3} />
                  <stop offset="95%" stopColor="#a78bfa" stopOpacity={0.02} />
                </linearGradient>
              </defs>
              <CartesianGrid
                strokeDasharray="3 3"
                stroke="hsl(var(--border))"
                vertical={false}
              />
              <XAxis
                dataKey="label"
                stroke="hsl(var(--muted-foreground))"
                fontSize={11}
                tickLine={false}
                minTickGap={24}
                interval={granularity === "hour" ? "preserveStartEnd" : "preserveEnd"}
              />
              <YAxis
                yAxisId="tokens"
                stroke="hsl(var(--muted-foreground))"
                fontSize={11}
                tickLine={false}
                axisLine={false}
                tickFormatter={(value) => fmtTokens(Number(value))}
                width={52}
                domain={[0, "auto"]}
              />
              <YAxis
                yAxisId="cost"
                orientation="right"
                stroke="hsl(var(--muted-foreground))"
                fontSize={11}
                tickLine={false}
                axisLine={false}
                tickFormatter={(value) => fmtAxisUsd(Number(value))}
                width={48}
                domain={[0, "auto"]}
              />
              <Tooltip
                contentStyle={{
                  backgroundColor: "hsl(var(--card) / 0.96)",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: "12px",
                  boxShadow: "0 18px 45px hsl(230 80% 2% / 0.35)",
                  fontSize: "12px",
                }}
                labelStyle={{ color: "hsl(var(--foreground))", marginBottom: "4px" }}
                cursor={{ stroke: "hsl(var(--muted-foreground))", strokeDasharray: "3 3" }}
                formatter={(value, name) => {
                  const v = typeof value === "number" ? value : Number(value) || 0;
                  const label = String(name ?? "");
                  return [label === "成本" ? fmtUsd(v) : fmtTokens(v), label];
                }}
              />
              <Legend content={<TrendLegend />} />
              <Area
                type="monotone"
                yAxisId="tokens"
                dataKey="cacheRead"
                name="缓存命中"
                stroke="#a78bfa"
                strokeWidth={2}
                fill="url(#cacheReadGradient)"
                fillOpacity={1}
                isAnimationActive={false}
              />
              <Line
                yAxisId="cost"
                type="monotone"
                dataKey="cost"
                name="成本"
                stroke="#fb7185"
                strokeWidth={2}
                strokeDasharray="4 4"
                dot={false}
                isAnimationActive={false}
              />
              <Line
                yAxisId="tokens"
                type="monotone"
                dataKey="cacheCreation"
                name="缓存创建"
                stroke="#fbbf24"
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
              <Line
                yAxisId="tokens"
                type="monotone"
                dataKey="input"
                name="输入"
                stroke="#22d3ee"
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
              <Line
                yAxisId="tokens"
                type="monotone"
                dataKey="output"
                name="输出"
                stroke="#34d399"
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
            </AreaChart>
          </ResponsiveContainer>
        ) : (
          <ResponsiveContainer width="100%" height="100%">
            <ComposedChart data={chartData} margin={{ top: 8, right: 4, left: 2, bottom: 22 }}>
              <defs>
                <linearGradient id="tokenBarGradient" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%" stopColor="#0ea5e9" stopOpacity={0.9} />
                  <stop offset="95%" stopColor="#22d3ee" stopOpacity={0.28} />
                </linearGradient>
              </defs>
              <CartesianGrid
                strokeDasharray="3 3"
                stroke="hsl(var(--border))"
                vertical={false}
              />
              <XAxis
                dataKey="label"
                stroke="hsl(var(--muted-foreground))"
                fontSize={11}
                tickLine={false}
                minTickGap={24}
                interval={granularity === "hour" ? "preserveStartEnd" : "preserveEnd"}
              />
              <YAxis
                yAxisId="tokens"
                stroke="hsl(var(--muted-foreground))"
                fontSize={11}
                tickLine={false}
                axisLine={false}
                tickFormatter={(value) => fmtTokens(Number(value))}
                width={52}
                domain={[0, "auto"]}
              />
              <YAxis
                yAxisId="cumulative"
                orientation="right"
                stroke="#f97316"
                fontSize={11}
                tickLine={false}
                axisLine={false}
                tickFormatter={(value) => fmtTokens(Number(value))}
                width={52}
                domain={[0, "auto"]}
              />
              <Tooltip
                contentStyle={{
                  backgroundColor: "hsl(var(--card) / 0.96)",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: "12px",
                  boxShadow: "0 18px 45px hsl(230 80% 2% / 0.22)",
                  fontSize: "12px",
                }}
                labelStyle={{ color: "hsl(var(--foreground))", marginBottom: "4px" }}
                cursor={{ fill: "hsl(var(--primary) / 0.055)" }}
                formatter={(value, name) => [fmtTokens(Number(value) || 0), String(name ?? "")]}
              />
              <Legend content={<CumulativeLegend />} />
              <Bar
                yAxisId="tokens"
                dataKey="total"
                name="时段真实 Tokens"
                fill="url(#tokenBarGradient)"
                radius={[6, 6, 1, 1]}
                maxBarSize={24}
                isAnimationActive={false}
              />
              <Line
                yAxisId="tokens"
                type="monotone"
                dataKey="cacheRead"
                name="缓存命中"
                stroke="#8b5cf6"
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
              <Line
                yAxisId="cumulative"
                type="monotone"
                dataKey="cumulative"
                name="累计真实 Tokens"
                stroke="#f97316"
                strokeWidth={2.5}
                dot={false}
                isAnimationActive={false}
              />
            </ComposedChart>
          </ResponsiveContainer>
        )}
      </div>
    </div>
  );
}
