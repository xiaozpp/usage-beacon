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
  zcode: { label: "ZCode", icon: "zcode", color: "text-slate-700 dark:text-white" },
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
    <section className="hero-panel p-4 md:p-5">
      <div className="relative z-10 flex flex-col gap-4">
        <div className="flex flex-col justify-between gap-4 md:flex-row md:items-center">
          <div className="flex items-center gap-3">
            <div className="hero-orb">
              {theme ? (
                <AppBrandIcon icon={theme.icon} name={theme.label} size={20} className={theme.color} />
              ) : (
                <Zap className="h-5 w-5 text-primary" />
              )}
            </div>
            <div>
              <div className="hero-kicker mb-2 flex items-center gap-1.5">
                {theme && <span className={cn("font-semibold", theme.color)}>{theme.label}</span>}
                {theme && <span className="text-muted-foreground/40">•</span>}
                <span>真实消耗 Tokens</span>
              </div>
              <div className="flex items-baseline gap-2">
                <span className="hero-primary-value tabular-nums">
                  {isLoading ? "..." : summary ? fmtInt(summary.realTotalTokens) : "-"}
                </span>
                {summary && !isLoading && (
                  <span className="hero-value-chip font-medium">
                    ≈ {fmtTokens(summary.realTotalTokens)}
                  </span>
                )}
              </div>
            </div>
          </div>

          <div className="hero-stat-group">
            <div className="flex flex-col">
              <span className="hero-stat-label">总请求数</span>
              <span className="hero-stat-value tabular-nums">
                <Zap className="h-3.5 w-3.5 text-blue-500" />
                {isLoading ? "..." : summary ? fmtInt(summary.totalRequests) : "-"}
              </span>
            </div>
            <div className="hero-stat-divider" />
            <div className="flex flex-col">
              <span className="hero-stat-label">总成本</span>
              <span className="hero-stat-value tabular-nums text-emerald-300">
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
          <div className="metric-tile col-span-2 flex flex-col justify-center lg:col-span-1">
            <div className="mb-2 flex items-center justify-between text-[11px]">
              <span className="metric-tile-label mb-0">缓存命中率</span>
              <span className="font-bold tabular-nums text-emerald-300">
                {summary && !isLoading ? fmtPercent(cacheHitRate) : "-"}
              </span>
            </div>
            <div className="progress-track">
              <div
                className="progress-fill absolute inset-y-0 left-0 rounded-full transition-[width] duration-500"
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
    <div className="metric-tile">
      <div className="metric-tile-label">
        <span className={accent}>{icon}</span>
        <span>{label}</span>
      </div>
      <div className={cn("text-sm font-semibold tabular-nums", muted && "text-muted-foreground")}>
        {isLoading ? "..." : value}
      </div>
    </div>
  );
}
