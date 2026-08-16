import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import type { UsageBreakdownStats } from "../types/usage";
import { fmtDateTime, fmtInt, fmtTokens, fmtUsd } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { AppBrandIcon, type AppBrandIconName } from "./AppBrandIcon";

interface Props {
  data: UsageBreakdownStats[] | undefined;
  isLoading: boolean;
  title: string;
  label: string;
  emptyMessage: string;
  mode: "project" | "session";
}

const SOURCE_ICONS: Record<string, AppBrandIconName> = {
  claude: "claude",
  codex: "openai",
  gemini: "gemini",
  grokbuild: "grok",
  opencode: "opencode",
  zcode: "zcode",
  deepseek_harness: "deepseek",
  hermes: "hermes",
};

const PROJECT_PAGE_SIZE = 5;

export function UsageBreakdownTable({
  data,
  isLoading,
  title,
  label,
  emptyMessage,
  mode,
}: Props) {
  const { t } = useI18n();
  const rows = data ?? [];
  const [page, setPage] = useState(1);
  const isPaginated = mode === "project";
  const totalPages = isPaginated
    ? Math.max(1, Math.ceil(rows.length / PROJECT_PAGE_SIZE))
    : 1;
  const visibleRows = isPaginated
    ? rows.slice((page - 1) * PROJECT_PAGE_SIZE, page * PROJECT_PAGE_SIZE)
    : rows;
  const maxCost = Math.max(...rows.map((row) => Number(row.totalCost) || 0), 0);

  useEffect(() => {
    setPage(1);
  }, [data, mode]);

  useEffect(() => {
    setPage((current) => Math.min(current, totalPages));
  }, [totalPages]);

  return (
    <div className="data-panel min-w-0">
      <div className="data-panel-header">
        <div className="panel-kicker">BREAKDOWN / {label.toUpperCase()}</div>
        <h2 className="panel-title">{title}</h2>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full min-w-[520px] text-sm">
          <thead className="table-head">
            <tr className="border-b border-border text-left">
              <th className="px-4 py-2 font-medium">{label}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.requests")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.realTokens")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.totalCost")}</th>
            </tr>
          </thead>
          <tbody>
            {isLoading ? (
              <tr>
                <td colSpan={4} className="px-4 py-8 text-center text-muted-foreground">
                  {t("common.loading")}
                </td>
              </tr>
            ) : rows.length === 0 ? (
              <tr>
                <td colSpan={4} className="px-4 py-8 text-center text-muted-foreground">
                  {emptyMessage}
                </td>
              </tr>
            ) : (
              visibleRows.map((row, index) => {
                const cost = Number(row.totalCost) || 0;
                const share = maxCost > 0 ? Math.max(4, (cost / maxCost) * 100) : 4;
                const sessionLabel = `${label} ${String(index + 1).padStart(2, "0")}`;
                const sourceIcon = SOURCE_ICONS[row.appType];
                const sourceLine =
                  mode === "project"
                    ? t("breakdown.projectSource", { source: row.sourceName })
                    : `${row.sourceName} · ${fmtDateTime(row.latestAt)}`;
                return (
                  <tr key={row.key} className="table-row border-b last:border-0">
                    <td
                      className="max-w-[260px] px-4 py-2.5"
                      title={
                        mode === "project"
                          ? t("breakdown.projectSourceTitle", { source: row.sourceName })
                          : undefined
                      }
                    >
                      <div className="flex min-w-0 items-center gap-1.5">
                        {sourceIcon && (
                          <AppBrandIcon
                            icon={sourceIcon}
                            name={row.sourceName}
                            size={15}
                          />
                        )}
                        <div className="truncate font-medium">
                          {mode === "session" ? sessionLabel : row.key}
                        </div>
                      </div>
                      {mode === "session" ? (
                        <div className="mt-1 truncate text-[0.68rem] text-muted-foreground">
                          {sourceLine}
                        </div>
                      ) : (
                        <div className="mt-1 truncate text-[0.68rem] text-muted-foreground">
                          {sourceLine} · {fmtDateTime(row.latestAt)}
                        </div>
                      )}
                      <div className="mt-1 h-1 overflow-hidden rounded-full bg-primary/10">
                        <div
                          className="h-full rounded-full bg-primary/60 transition-all"
                          style={{ width: `${share}%` }}
                        />
                      </div>
                    </td>
                    <td className="px-4 py-2.5 text-right tabular-nums">
                      {fmtInt(row.requestCount)}
                    </td>
                    <td className="px-4 py-2.5 text-right tabular-nums">
                      {fmtTokens(row.totalTokens)}
                    </td>
                    <td className="px-4 py-2.5 text-right tabular-nums">
                      <div
                        className={
                          row.unpricedRequests > 0 && Number(row.totalCost) === 0
                            ? "font-semibold text-amber-600 dark:text-amber-400"
                            : undefined
                        }
                      >
                        {row.unpricedRequests > 0 && Number(row.totalCost) === 0
                          ? t("breakdown.unpriced")
                          : fmtUsd(row.totalCost)}
                      </div>
                      <div className="text-[0.68rem] text-muted-foreground">
                        {row.unpricedRequests > 0
                          ? t("breakdown.unpricedCount", { count: row.unpricedRequests })
                          : `${fmtUsd(row.avgCostPerRequest)} / ${t("stats.requests").toLowerCase()}`}
                      </div>
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>
      {isPaginated && totalPages > 1 && (
        <div className="flex items-center justify-between border-t border-border px-4 py-3">
          <span className="text-xs text-muted-foreground">
            {t("logs.page", { page, pages: totalPages })}
          </span>
          <div className="flex gap-1">
            <button
              onClick={() => setPage((current) => Math.max(1, current - 1))}
              disabled={page <= 1}
              className="rounded p-1 hover:bg-muted disabled:opacity-30"
              aria-label={t("logs.previous")}
              title={t("logs.previous")}
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <button
              onClick={() => setPage((current) => Math.min(totalPages, current + 1))}
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
    </div>
  );
}
