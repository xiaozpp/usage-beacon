import type { ModelStats as ModelStatsType, RadarIqPoint } from "../types/usage";
import { fmtInt, fmtUsd, fmtTokens, isPotentiallyUnpriced } from "../lib/format";
import { useI18n } from "../lib/i18n";
import { useCodexRadar } from "../lib/hooks";

interface Props {
  data: ModelStatsType[] | undefined;
  isLoading: boolean;
}

export function ModelStatsTable({ data, isLoading }: Props) {
  const { t } = useI18n();
  const rows = data ?? [];
  const radar = useCodexRadar();
  const radarPoints = (radar.data?.iq?.points ?? []).filter(
    (point) => point.total >= 30 && point.averagePriceUsd !== null,
  );

  return (
    <div className="data-panel">
      <div className="data-panel-header">
        <h2 className="panel-title mt-0">{t("stats.modelTitle")}</h2>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="table-head">
            <tr className="border-b border-border text-left">
              <th className="px-4 py-2 font-medium">{t("stats.model")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.requests")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.realTokens")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.totalCost")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.radarAvg")}</th>
              <th className="px-4 py-2 text-right font-medium">{t("stats.localAvg")}</th>
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
              rows.map((row) => {
                const radarRange = getRadarPriceRange(row.model, radarPoints);
                return (
                  <tr
                    key={row.model}
                    className="table-row border-b last:border-0"
                  >
                    <td className="px-4 py-2 font-mono text-xs">{row.model}</td>
                    <td className="px-4 py-2 text-right tabular-nums">{fmtInt(row.requestCount)}</td>
                    <td className="px-4 py-2 text-right tabular-nums">
                      {fmtTokens(row.totalTokens)}
                    </td>
                    <td className="px-4 py-2 text-right tabular-nums">
                      {isPotentiallyUnpriced(row.totalCost, row.totalTokens)
                        ? t("breakdown.unpriced")
                        : fmtUsd(row.totalCost)}
                    </td>
                    <td
                      className="radar-price-cell px-4 py-2 text-right tabular-nums"
                      title={radarRange ? t("stats.radarTitle") : t("stats.radarNoSample")}
                    >
                      {radarRange ?? "—"}
                    </td>
                    <td className="px-4 py-2 text-right tabular-nums text-muted-foreground">
                      {isPotentiallyUnpriced(row.totalCost, row.totalTokens)
                        ? t("breakdown.unpriced")
                        : fmtUsd(row.avgCostPerRequest)}
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function getRadarPriceRange(model: string, points: RadarIqPoint[]) {
  const modelKey = model.toLowerCase().split("/").pop() ?? model.toLowerCase();
  const prices = points
    .filter((point) => point.model.toLowerCase() === modelKey)
    .map((point) => point.averagePriceUsd)
    .filter((price): price is number => price !== null)
    .sort((left, right) => left - right);
  if (!prices.length) return null;
  const low = prices[0];
  const high = prices[prices.length - 1];
  return low === high ? fmtUsd(low) : `${fmtUsd(low)}–${fmtUsd(high)}`;
}
