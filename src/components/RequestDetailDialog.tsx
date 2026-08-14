import { X } from "lucide-react";
import type { RequestLogDetail } from "../types/usage";
import { useRequestDetail } from "../lib/hooks";
import { fmtDateTime, fmtLatency, fmtTokens, fmtUsd } from "../lib/format";

interface Props {
  requestId: string | null;
  onClose: () => void;
}

export function RequestDetailDialog({ requestId, onClose }: Props) {
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
            <h2 className="panel-title">请求详情</h2>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            aria-label="关闭"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 内容 */}
        <div className="p-4">
          {isLoading ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              加载中...
            </div>
          ) : error ? (
            <div className="py-8 text-center text-sm text-red-500">
              加载失败: {String(error)}
            </div>
          ) : !data ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              未找到数据
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
  return (
    <div className="space-y-4 text-sm">
      <Section title="基本信息">
        <Field label="请求 ID" value={detail.requestId} mono />
        <Field label="Provider" value={detail.providerName} />
        <Field label="App Type" value={detail.appType} />
        <Field label="数据来源" value={detail.dataSource} />
        <Field label="时间" value={fmtDateTime(detail.createdAt)} />
      </Section>

      <Section title="模型与计费">
        <Field label="模型" value={detail.model} mono />
        {detail.requestModel && (
          <Field label="请求模型" value={detail.requestModel} mono />
        )}
        {detail.pricingModel && (
          <Field label="计价模型" value={detail.pricingModel} mono />
        )}
        <Field label="成本倍率" value={detail.costMultiplier} />
        <Field label="流式" value={detail.isStreaming ? "是" : "否"} />
      </Section>

      <Section title="Token 使用量">
        <Field label="新增输入" value={fmtTokens(detail.freshInputTokens)} />
        <Field label="原始输入" value={fmtTokens(detail.inputTokens)} />
        <Field label="输出" value={fmtTokens(detail.outputTokens)} />
        <Field label="缓存读" value={fmtTokens(detail.cacheReadTokens)} />
        <Field label="缓存写" value={fmtTokens(detail.cacheCreationTokens)} />
      </Section>

      <Section title="成本明细 (USD)">
        <Field label="输入成本" value={fmtUsd(detail.inputCostUsd)} />
        <Field label="输出成本" value={fmtUsd(detail.outputCostUsd)} />
        <Field label="缓存读成本" value={fmtUsd(detail.cacheReadCostUsd)} />
        <Field label="缓存写成本" value={fmtUsd(detail.cacheCreationCostUsd)} />
        <Field
          label="总成本"
          value={fmtUsd(detail.totalCostUsd)}
          emphasize
        />
      </Section>

      <Section title="响应状态">
        <Field label="状态码" value={String(detail.statusCode)} />
        <Field label="耗时" value={fmtLatency(detail.latencyMs)} />
        {detail.sessionId && (
          <Field label="会话 ID" value={detail.sessionId} mono />
        )}
        {detail.errorMessage && (
          <Field label="错误信息" value={detail.errorMessage} />
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
