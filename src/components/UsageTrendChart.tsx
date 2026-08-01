import {
  Area,
  AreaChart,
  CartesianGrid,
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
}

export function UsageTrendChart({ data, isLoading }: Props) {
  const chartData = (data ?? []).map((d) => ({
    date: d.date.slice(5), // MM-DD
    cost: parseFloat(d.totalCost),
    tokens: d.totalTokens,
    requests: d.requestCount,
  }));

  return (
    <div className="rounded-lg border border-border bg-card p-4">
      <div className="mb-4 flex items-center justify-between">
        <h2 className="text-sm font-semibold">成本趋势</h2>
        {isLoading && <span className="text-xs text-muted-foreground">加载中...</span>}
      </div>
      <div className="h-64 w-full">
        {chartData.length === 0 ? (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            暂无数据
          </div>
        ) : (
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={chartData} margin={{ top: 5, right: 5, left: 0, bottom: 0 }}>
              <defs>
                <linearGradient id="costGradient" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%" stopColor="#10b981" stopOpacity={0.8} />
                  <stop offset="95%" stopColor="#10b981" stopOpacity={0} />
                </linearGradient>
              </defs>
              <CartesianGrid strokeDasharray="3 3" stroke="hsl(var(--border))" />
              <XAxis dataKey="date" stroke="hsl(var(--muted-foreground))" fontSize={11} />
              <YAxis stroke="hsl(var(--muted-foreground))" fontSize={11} />
              <Tooltip
                contentStyle={{
                  backgroundColor: "hsl(var(--card))",
                  border: "1px solid hsl(var(--border))",
                  borderRadius: "8px",
                  fontSize: "12px",
                }}
                formatter={(value, name) => {
                  const v = typeof value === "number" ? value : Number(value) || 0;
                  const n = String(name ?? "");
                  if (n === "cost") return [fmtUsd(v), "成本"];
                  return [fmtTokens(v), "Tokens"];
                }}
              />
              <Area
                type="monotone"
                dataKey="cost"
                stroke="#10b981"
                strokeWidth={2}
                fill="url(#costGradient)"
              />
            </AreaChart>
          </ResponsiveContainer>
        )}
      </div>
    </div>
  );
}
