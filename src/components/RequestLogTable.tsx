import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { LogFilters, RequestLogDetail } from "../types/usage";
import { fmtDateTime, fmtUsd, fmtTokens } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useRequestLogs } from "../lib/hooks";
import { RequestDetailDialog } from "./RequestDetailDialog";

interface Props {
  filters: LogFilters;
  refetchMs?: number;
}

export function RequestLogTable({ filters, refetchMs = 30000 }: Props) {
  const { t } = useI18n();
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
          <h2 className="panel-title">{t("logs.title")}</h2>
        </div>
        <span className="text-xs text-muted-foreground">{t("logs.total", { count: total })}</span>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="table-head">
            <tr className="border-b border-border text-left">
              <th className="px-4 py-2 font-medium">{t("logs.time")}</th>
              <th className="px-4 py-2 font-medium">{t("logs.model")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("logs.freshInput")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("logs.output")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("logs.cacheRead")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("logs.cost")}</th>
              <th className="px-4 py-2 font-medium">{t("logs.status")}</th>
            </tr>
          </thead>
          <tbody>
            {isLoading ? (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-muted-foreground">
                  {t("common.loading")}
                </td>
              </tr>
            ) : error ? (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-red-500">
                  {t("logs.loadError", {
                    error: error instanceof Error ? error.message : String(error),
                  })}
                </td>
              </tr>
            ) : rows.length === 0 ? (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-muted-foreground">
                  {t("common.noData")}
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
            {t("logs.page", { page, pages: totalPages })}
          </span>
          <div className="flex gap-1">
            <button
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              disabled={page <= 1}
              className="rounded p-1 hover:bg-muted disabled:opacity-30"
              aria-label={t("logs.previous")}
              title={t("logs.previous")}
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <button
              onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
              disabled={page >= totalPages}
              className="rounded p-1 hover:bg-muted disabled:opacity-30"
              aria-label={t("logs.next")}
              title={t("logs.next")}
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
