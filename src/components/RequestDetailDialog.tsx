import { X } from "lucide-react";
import type { RequestLogDetail } from "../types/usage";
import { useRequestDetail } from "../lib/hooks";
import { fmtDateTime, fmtLatency, fmtTokens, fmtUsd } from "../lib/format";
import { useI18n } from "../lib/i18n";

interface Props {
  requestId: string | null;
  onClose: () => void;
}

export function RequestDetailDialog({ requestId, onClose }: Props) {
  const { t } = useI18n();
  const { data, isLoading, error } = useRequestDetail(requestId);

  if (!requestId) return null;

  return (
    <div
      className="dialog-backdrop fixed inset-0 z-50 flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="dialog-panel max-h-[85vh] w-full max-w-2xl overflow-y-auto rounded-2xl border bg-card shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 标题栏 */}
        <div className="flex items-center justify-between border-b border-border px-4 py-3.5">
          <div>
            <div className="panel-kicker">EVENT INSPECTOR</div>
            <h2 className="panel-title">{t("detail.title")}</h2>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            aria-label={t("detail.close")}
            title={t("detail.close")}
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 内容 */}
        <div className="p-4">
          {isLoading ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              {t("common.loading")}
            </div>
          ) : error ? (
            <div className="py-8 text-center text-sm text-red-500">
              {t("detail.loadError", { error: String(error) })}
            </div>
          ) : !data ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              {t("detail.notFound")}
            </div>
          ) : (
            <DetailBody detail={data} />
          )}
        </div>
      </div>
    </div>
  );
}

function DetailBody({ detail }: { detail: RequestLogDetail }) {
  const { t } = useI18n();
  return (
    <div className="space-y-4 text-sm">
      <Section title={t("detail.basic")}>
        <Field label={t("detail.requestId")} value={detail.requestId} mono />
        <Field label={t("detail.provider")} value={detail.providerName} />
        <Field label={t("detail.appType")} value={detail.appType} />
        <Field label={t("detail.dataSource")} value={detail.dataSource} />
        <Field label={t("detail.time")} value={fmtDateTime(detail.createdAt)} />
      </Section>

      <Section title={t("detail.modelBilling")}>
        <Field label={t("detail.model")} value={detail.model} mono />
        {detail.requestModel && (
          <Field label={t("detail.requestModel")} value={detail.requestModel} mono />
        )}
        {detail.pricingModel && (
          <Field label={t("detail.pricingModel")} value={detail.pricingModel} mono />
        )}
        <Field label={t("detail.costMultiplier")} value={detail.costMultiplier} />
        <Field label={t("detail.streaming")} value={detail.isStreaming ? t("common.yes") : t("common.no")} />
      </Section>

      <Section title={t("detail.tokenUsage")}>
        <Field label={t("hero.input")} value={fmtTokens(detail.freshInputTokens)} />
        <Field label={t("detail.rawInput")} value={fmtTokens(detail.inputTokens)} />
        <Field label={t("hero.output")} value={fmtTokens(detail.outputTokens)} />
        <Field label={t("hero.cacheRead")} value={fmtTokens(detail.cacheReadTokens)} />
        <Field label={t("hero.cacheCreation")} value={fmtTokens(detail.cacheCreationTokens)} />
      </Section>

      <Section title={t("detail.costDetails")}>
        <Field label={t("detail.inputCost")} value={fmtUsd(detail.inputCostUsd)} />
        <Field label={t("detail.outputCost")} value={fmtUsd(detail.outputCostUsd)} />
        <Field label={t("detail.cacheReadCost")} value={fmtUsd(detail.cacheReadCostUsd)} />
        <Field label={t("detail.cacheWriteCost")} value={fmtUsd(detail.cacheCreationCostUsd)} />
        <Field
          label={t("detail.totalCost")}
          value={fmtUsd(detail.totalCostUsd)}
          emphasize
        />
      </Section>

      <Section title={t("detail.responseStatus")}>
        <Field label={t("detail.statusCode")} value={String(detail.statusCode)} />
        <Field label={t("detail.latency")} value={fmtLatency(detail.latencyMs)} />
        {detail.sessionId && (
          <Field label={t("detail.sessionId")} value={detail.sessionId} mono />
        )}
        {detail.errorMessage && (
          <Field label={t("detail.errorMessage")} value={detail.errorMessage} />
        )}
      </Section>
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <h3 className="panel-kicker mb-2">
        {title}
      </h3>
      <div className="grid grid-cols-1 gap-1 sm:grid-cols-2">{children}</div>
    </div>
  );
}

function Field({
  label,
  value,
  mono,
  emphasize,
}: {
  label: string;
  value: string;
  mono?: boolean;
  emphasize?: boolean;
  }) {
  return (
    <div className="flex items-start justify-between gap-2 rounded-lg border border-border/60 bg-background/40 px-2.5 py-2">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span
        className={`text-right text-xs ${mono ? "font-mono" : ""} ${
          emphasize ? "font-semibold text-emerald-500" : ""
        } break-all`}
      >
        {value}
      </span>
    </div>
  );
}
