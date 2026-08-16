import type { ProviderStats as ProviderStatsType } from "../types/usage";
import { fmtInt, fmtLatency, fmtPercent, fmtTokens, fmtUsd } from "../lib/format";
import { useI18n } from "../lib/i18n";

interface Props {
  data: ProviderStatsType[] | undefined;
  isLoading: boolean;
}

export function ProviderStatsTable({ data, isLoading }: Props) {
  const { t } = useI18n();
  const rows = data ?? [];

  return (
    <div className="data-panel">
      <div className="data-panel-header">
        <h2 className="panel-title mt-0">{t("stats.providerTitle")}</h2>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="table-head">
            <tr className="border-b border-border text-left">
              <th className="px-4 py-2 font-medium">{t("stats.provider")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.requests")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.realTokens")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.totalCost")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.successRate")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.avgLatency")}</th>
            </tr>
          </thead>
          <tbody>
            {isLoading ? (
              <tr>
                <td colSpan={6} className="px-4 py-8 text-center text-muted-foreground">
                  {t("stats.loading")}
                </td>
              </tr>
            ) : rows.length === 0 ? (
              <tr>
                <td colSpan={6} className="px-4 py-8 text-center text-muted-foreground">
                  {t("stats.noData")}
                </td>
              </tr>
            ) : (
              rows.map((row) => (
                <tr
                  key={`${row.providerId}-${row.providerName}`}
                  className="table-row border-b last:border-0"
                >
                  <td className="px-4 py-2 font-medium">{row.providerName}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmtInt(row.requestCount)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmtTokens(row.totalTokens)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">{fmtUsd(row.totalCost)}</td>
                  <td className="px-4 py-2 text-right tabular-nums">
                    {fmtPercent(row.successRate)}
                  </td>
                  <td className="px-4 py-2 text-right tabular-nums text-muted-foreground">
                    {row.avgLatencyMs === null
                      ? t("common.notAvailable")
                      : fmtLatency(row.avgLatencyMs)}
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
