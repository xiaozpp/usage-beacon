import {
  Area,
  AreaChart,
  CartesianGrid,
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
  input: number;
  output: number;
  cacheCreation: number;
  cacheRead: number;
}

const SERIES = [
  { key: "cost", label: "成本", color: "#ff375f", dashed: true },
  { key: "cacheCreation", label: "缓存创建", color: "#f97316", dashed: false },
  { key: "cacheRead", label: "缓存命中", color: "#8b5cf6", dashed: false },
  { key: "input", label: "输入", color: "#3b82f6", dashed: false },
  { key: "output", label: "输出", color: "#10b981", dashed: false },
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
    input: 0,
    output: 0,
    cacheCreation: 0,
    cacheRead: 0,
  };
}

function toTrendPoint(stats: DailyStats, bucket: string, label: string): TrendPoint {
  return {
    bucket,
    label,
    cost: Number.parseFloat(stats.totalCost) || 0,
    input: stats.inputTokens,
    output: stats.outputTokens,
    cacheCreation: stats.cacheCreationTokens,
    cacheRead: stats.cacheReadTokens,
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
    return {
      granularity,
      data: data.map((stats) => {
        const parts = stats.date.slice(0, 10).split("-").map(Number);
        const month = parts[1] ?? 0;
        const day = parts[2] ?? 0;
        const label = month && day ? `${pad(month)}/${pad(day)}` : stats.date;
        return toTrendPoint(stats, getSourceBucketKey(stats.date, granularity), label);
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

  return { data: points, granularity };
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

export function UsageTrendChart({
  data,
  isLoading,
  startDate,
  endDate,
  rangeLabel,
}: Props) {
  const chart = data ? buildChartData(data, startDate, endDate) : null;
  const chartData = chart?.data ?? [];
  const granularity = chart?.granularity ?? getGranularity(startDate, endDate);

  return (
    <div className="rounded-lg border border-border bg-card p-4 md:p-5">
      <div className="mb-3 flex items-center justify-between">
        <h2 className="text-sm font-semibold">使用趋势</h2>
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          {rangeLabel && <span>{rangeLabel}</span>}
          {isLoading && <span>加载中...</span>}
        </div>
      </div>
      <div className="h-[360px] w-full md:h-[400px]">
        {chartData.length === 0 ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {isLoading ? "加载中..." : "暂无数据"}
          </div>
        ) : (
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={chartData} margin={{ top: 8, right: 4, left: 2, bottom: 22 }}>
              <defs>
                <linearGradient id="cacheReadGradient" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%" stopColor="#8b5cf6" stopOpacity={0.28} />
                  <stop offset="95%" stopColor="#8b5cf6" stopOpacity={0.02} />
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
                  backgroundColor: "hsl(var(--card))",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: "8px",
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
                stroke="#8b5cf6"
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
                stroke="#ff375f"
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
                stroke="#f97316"
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
              <Line
                yAxisId="tokens"
                type="monotone"
                dataKey="input"
                name="输入"
                stroke="#3b82f6"
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
              <Line
                yAxisId="tokens"
                type="monotone"
                dataKey="output"
                name="输出"
                stroke="#10b981"
                strokeWidth={2}
                dot={false}
                isAnimationActive={false}
              />
            </AreaChart>
          </ResponsiveContainer>
        )}
      </div>
    </div>
  );
}
