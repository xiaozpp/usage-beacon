import type { ProviderStats as ProviderStatsType } from "../types/usage";
import { fmtInt, fmtLatency, fmtPercent, fmtTokens, fmtUsd } from "../lib/format";

interface Props {
  data: ProviderStatsType[] | undefined;
  isLoading: boolean;
}

export function ProviderStatsTable({ data, isLoading }: Props) {
  const rows = data ?? [];

  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="border-b border-border px-4 py-3">
        <h2 className="text-sm font-semibold">供应商统计</h2>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-left text-xs text-muted-foreground">
              <th className="px-4 py-2 font-medium">供应商</th>
              <th className="px-4 py-2 text-right font-medium">请求数</th>
              <th className="px-4 py-2 text-right font-medium">Tokens</th>
              <th className="px-4 py-2 text-right font-medium">总成本</th>
              <th className="px-4 py-2 text-right font-medium">成功率</th>
              <th className="px-4 py-2 text-right font-medium">平均延迟</th>
            </tr>
          </thead>
          <tbody>
            {isLoading ? (
              <tr>
                <td colSpan={6} className="px-4 py-8 text-center text-muted-foreground">
                  加载中...
                </td>
              </tr>
            ) : rows.length === 0 ? (
              <tr>
                <td colSpan={6} className="px-4 py-8 text-center text-muted-foreground">
                  暂无数据
                </td>
              </tr>
            ) : (
              rows.map((row) => (
                <tr
                  key={`${row.providerId}-${row.providerName}`}
                  className="border-b border-border/50 last:border-0 hover:bg-muted/50"
                >
                  <td className="px-4 py-2 font-medium">{row.providerName}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmtInt(row.requestCount)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmtTokens(row.totalTokens)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmtUsd(row.totalCost)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">
                    {fmtPercent(row.successRate)}
                  </td>
                  <td className="px-4 py-2 text-right tabular-nums text-muted-foreground">
                    {fmtLatency(row.avgLatencyMs)}
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
