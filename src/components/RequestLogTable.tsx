import { useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { LogFilters, RequestLogDetail } from "../types/usage";
import { fmtDateTime, fmtUsd, fmtTokens } from "../lib/format";
import { useRequestLogs } from "../lib/hooks";
import { RequestDetailDialog } from "./RequestDetailDialog";

interface Props {
  filters: LogFilters;
  refetchMs?: number;
}

export function RequestLogTable({ filters, refetchMs = 30000 }: Props) {
  const [page, setPage] = useState(1);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const pageSize = 20;
  const { data, isLoading, error } = useRequestLogs(filters, page, pageSize, refetchMs);

  const rows = data?.data ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  return (
    <div className="rounded-lg border border-border bg-card">
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <h2 className="text-sm font-semibold">请求日志</h2>
        <span className="text-xs text-muted-foreground">共 {total} 条</span>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border text-left text-xs text-muted-foreground">
              <th className="px-4 py-2 font-medium">时间</th>
              <th className="px-4 py-2 font-medium">模型</th>
              <th className="px-4 py-2 text-right font-medium">输入</th>
              <th className="px-4 py-2 text-right font-medium">输出</th>
              <th className="px-4 py-2 text-right font-medium">缓存读</th>
              <th className="px-4 py-2 text-right font-medium">成本</th>
              <th className="px-4 py-2 font-medium">状态</th>
            </tr>
          </thead>
          <tbody>
            {isLoading ? (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-muted-foreground">
                  加载中...
                </td>
              </tr>
            ) : error ? (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-red-500">
                  加载失败：{error instanceof Error ? error.message : String(error)}
                </td>
              </tr>
            ) : rows.length === 0 ? (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-muted-foreground">
                  暂无数据
                </td>
              </tr>
            ) : (
              rows.map((row: RequestLogDetail) => (
                <tr
                  key={row.requestId}
                  onClick={() => setSelectedId(row.requestId)}
                  className="cursor-pointer border-b border-border/50 last:border-0 hover:bg-muted/50"
                >
                  <td className="px-4 py-2 text-xs text-muted-foreground">
                    {fmtDateTime(row.createdAt)}
                  </td>
                  <td className="px-4 py-2 font-mono text-xs">{row.model}</td>
                  <td className="px-4 py-2 text-right tabular-nums">
                    {fmtTokens(row.inputTokens)}
                  </td>
                  <td className="px-4 py-2 text-right tabular-nums">
                    {fmtTokens(row.outputTokens)}
                  </td>
                  <td className="px-4 py-2 text-right tabular-nums text-amber-500">
                    {fmtTokens(row.cacheReadTokens)}
                  </td>
                  <td className="px-4 py-2 text-right tabular-nums">
                    {fmtUsd(row.totalCostUsd)}
                  </td>
                  <td className="px-4 py-2">
                    <StatusBadge code={row.statusCode} />
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      {/* 分页 */}
      {total > pageSize && (
        <div className="flex items-center justify-between border-t border-border px-4 py-2">
          <span className="text-xs text-muted-foreground">
            第 {page} / {totalPages} 页
          </span>
          <div className="flex gap-1">
            <button
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              disabled={page <= 1}
              className="rounded p-1 hover:bg-muted disabled:opacity-30"
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <button
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
              disabled={page >= totalPages}
              className="rounded p-1 hover:bg-muted disabled:opacity-30"
            >
              <ChevronRight className="h-4 w-4" />
            </button>
          </div>
        </div>
      )}

      <RequestDetailDialog
        requestId={selectedId}
        onClose={() => setSelectedId(null)}
      />
    </div>
  );
}

function StatusBadge({ code }: { code: number }) {
  const ok = code >= 200 && code < 300;
  return (
    <span
      className={
        ok
          ? "rounded bg-emerald-500/10 px-1.5 py-0.5 text-xs text-emerald-500"
          : "rounded bg-red-500/10 px-1.5 py-0.5 text-xs text-red-500"
      }
    >
      {code}
    </span>
  );
}
