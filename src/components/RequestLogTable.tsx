import { useEffect, useState } from "react";
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
  const pageSize = 10;
  const { data, isLoading, error } = useRequestLogs(filters, page, pageSize, refetchMs);

  const rows = data?.data ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / pageSize));

  useEffect(() => {
    setPage(1);
  }, [
    filters.appType,
    filters.providerName,
    filters.model,
    filters.deviceId,
    filters.statusCode,
    filters.startDate,
    filters.endDate,
  ]);

  useEffect(() => {
    setPage((current) => Math.min(current, totalPages));
  }, [totalPages]);

  return (
    <div className="data-panel">
      <div className="data-panel-header flex items-center justify-between gap-3">
        <div>
          <div className="panel-kicker">EVENT STREAM</div>
          <h2 className="panel-title">请求日志</h2>
        </div>
        <span className="text-xs text-muted-foreground">共 {total} 条</span>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="table-head">
            <tr className="border-b border-border text-left">
              <th className="px-4 py-2 font-medium">时间</th>
              <th className="px-4 py-2 font-medium">模型</th>
              <th className="px-4 py-2 text-right font-medium">新增输入</th>
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
                  className="table-row cursor-pointer border-b last:border-0"
                >
                  <td className="px-4 py-2 text-xs text-muted-foreground">
                    {fmtDateTime(row.createdAt)}
                  </td>
                  <td className="px-4 py-2 font-mono text-xs">{row.model}</td>
                  <td className="px-4 py-2 text-right tabular-nums">
                    {fmtTokens(row.freshInputTokens)}
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
        <div className="flex items-center justify-between border-t border-border px-4 py-3">
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
          ? "status-badge bg-emerald-400/10 text-emerald-300"
          : "status-badge bg-red-400/10 text-red-300"
      }
    >
      {code}
    </span>
  );
}
