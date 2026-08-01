import type { ReactNode } from "react";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  Database,
  Sparkles,
  Zap,
} from "lucide-react";
import type { UsageSummary } from "../types/usage";
import { fmtInt, fmtPercent, fmtTokens, fmtUsd } from "../lib/format";
import { cn } from "../lib/utils";
import { AppBrandIcon, type AppBrandIconName } from "./AppBrandIcon";

interface Props {
  summary: UsageSummary | undefined;
  isLoading: boolean;
  appType?: string | null;
}

const APP_THEME: Record<string, { label: string; icon: AppBrandIconName; color: string }> = {
  claude: { label: "Claude", icon: "claude", color: "text-amber-500" },
  codex: { label: "Codex", icon: "openai", color: "text-slate-600 dark:text-slate-300" },
  gemini: { label: "Gemini", icon: "gemini", color: "text-sky-500" },
  grokbuild: { label: "Grok Build", icon: "grok", color: "text-rose-500" },
  opencode: { label: "OpenCode", icon: "opencode", color: "text-violet-500" },
};

export function UsageHero({ summary, isLoading, appType }: Props) {
  const theme = appType ? APP_THEME[appType] : undefined;
  const cacheHitRate = summary?.cacheHitRate ?? 0;
  const cacheWriteUnavailable =
    Boolean(summary) &&
    summary?.cacheCreationTokens === 0 &&
    appType !== "claude" &&
    appType !== "opencode";

  return (
    <section className="rounded-xl border border-border bg-card/70 p-4 shadow-sm md:p-5">
      <div className="flex flex-col gap-4">
        <div className="flex flex-col justify-between gap-4 md:flex-row md:items-center">
          <div className="flex items-center gap-3">
            <div className="rounded-xl bg-primary/10 p-2.5">
              {theme ? (
                <AppBrandIcon icon={theme.icon} name={theme.label} size={20} className={theme.color} />
              ) : (
                <Zap className="h-5 w-5 text-primary" />
              )}
            </div>
            <div>
              <div className="mb-1 flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                {theme && <span className={cn("font-semibold", theme.color)}>{theme.label}</span>}
                {theme && <span className="text-muted-foreground/40">•</span>}
                <span>真实消耗 Tokens</span>
              </div>
              <div className="flex items-baseline gap-2">
                <span className="text-2xl font-bold leading-none tracking-tight tabular-nums md:text-3xl">
                  {isLoading ? "..." : summary ? fmtInt(summary.realTotalTokens) : "-"}
                </span>
                {summary && !isLoading && (
                  <span className="rounded-md bg-muted/60 px-1.5 py-0.5 text-xs font-medium text-muted-foreground">
                    ≈ {fmtTokens(summary.realTotalTokens)}
                  </span>
                )}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-5 rounded-xl border border-border/60 bg-background/60 px-4 py-2.5 shadow-sm">
            <div className="flex flex-col">
              <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                总请求数
              </span>
              <span className="flex items-center gap-1.5 text-sm font-semibold tabular-nums">
                <Zap className="h-3.5 w-3.5 text-blue-500" />
                {isLoading ? "..." : summary ? fmtInt(summary.totalRequests) : "-"}
              </span>
            </div>
            <div className="h-8 w-px bg-border/60" />
            <div className="flex flex-col">
              <span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
                总成本
              </span>
              <span className="text-sm font-semibold tabular-nums text-emerald-500">
                {isLoading ? "..." : summary ? fmtUsd(summary.totalCost) : "-"}
              </span>
            </div>
          </div>
        </div>

        <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
          <MiniStat
            icon={<ArrowDownToLine className="h-3.5 w-3.5" />}
            label="新增输入"
            value={summary ? fmtTokens(summary.inputTokens) : "-"}
            accent="text-blue-500"
            isLoading={isLoading}
          />
          <MiniStat
            icon={<ArrowUpFromLine className="h-3.5 w-3.5" />}
            label="Output"
            value={summary ? fmtTokens(summary.outputTokens) : "-"}
            accent="text-violet-500"
            isLoading={isLoading}
          />
          <MiniStat
            icon={<Database className="h-3.5 w-3.5" />}
            label="缓存创建"
            value={cacheWriteUnavailable ? "N/A" : summary ? fmtTokens(summary.cacheCreationTokens) : "-"}
            accent="text-amber-500"
            muted={cacheWriteUnavailable}
            isLoading={isLoading}
          />
          <MiniStat
            icon={<Sparkles className="h-3.5 w-3.5" />}
            label="缓存命中"
            value={summary ? fmtTokens(summary.cacheReadTokens) : "-"}
            accent="text-emerald-500"
            isLoading={isLoading}
          />
          <div className="col-span-2 flex flex-col justify-center rounded-xl border border-border/60 bg-background/40 p-3 shadow-sm lg:col-span-1">
            <div className="mb-2 flex items-center justify-between text-[11px]">
              <span className="font-medium text-muted-foreground">缓存命中率</span>
              <span className="font-bold tabular-nums text-emerald-500">
                {summary && !isLoading ? fmtPercent(cacheHitRate) : "-"}
              </span>
            </div>
            <div className="relative h-1.5 overflow-hidden rounded-full bg-muted/70">
              <div
                className="absolute inset-y-0 left-0 rounded-full bg-emerald-500 transition-[width] duration-500"
                style={{ width: `${Math.max(0, Math.min(100, cacheHitRate))}%` }}
              />
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function MiniStat({
  icon,
  label,
  value,
  accent,
  muted = false,
  isLoading,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  accent: string;
  muted?: boolean;
  isLoading: boolean;
}) {
  return (
    <div className="rounded-xl border border-border/60 bg-background/40 p-3 shadow-sm">
      <div className="mb-1.5 flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground">
        <span className={accent}>{icon}</span>
        <span>{label}</span>
      </div>
      <div className={cn("text-sm font-semibold tabular-nums", muted && "text-muted-foreground")}>
        {isLoading ? "..." : value}
      </div>
    </div>
  );
}
