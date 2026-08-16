import {
  AlertTriangle,
  CheckCircle2,
  CircleDashed,
  Clock3,
  Database,
} from "lucide-react";
import type { SyncSourceStatus } from "../types/usage";
import { useI18n } from "../lib/i18n";
import { cn } from "../lib/utils";
import { AppBrandIcon, type AppBrandIconName } from "./AppBrandIcon";

interface Props {
  sources: SyncSourceStatus[] | undefined;
}

const SOURCE_ICONS: Record<string, AppBrandIconName> = {
  claude: "claude",
  codex: "openai",
  gemini: "gemini",
  opencode: "opencode",
  zcode: "zcode",
  "grok build": "grok",
  "deepseek harness": "deepseek",
  hermes: "hermes",
};

export function SyncHealthPanel({ sources }: Props) {
  const { t } = useI18n();
  if (!sources?.length) return null;

  return (
    <section className="data-panel p-3 md:p-4">
      <div className="mb-3 flex items-end justify-between gap-3">
        <div>
          <div className="panel-kicker">SOURCES / HEALTH</div>
          <h2 className="panel-title">{t("status.syncHealthTitle")}</h2>
        </div>
        <div className="hidden items-center gap-1.5 text-xs text-muted-foreground sm:flex">
          <Database className="h-3.5 w-3.5" />
          {t("status.syncHealthSubtitle")}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2 md:grid-cols-3 xl:grid-cols-6">
        {sources.map((source) => {
          const icon = SOURCE_ICONS[source.name.toLowerCase()];
          const hasError = source.errors.length > 0;
          const hasDeferred = source.deferredFiles > 0;
          const state = hasError ? "error" : hasDeferred ? "deferred" : source.detected ? "ok" : "idle";

          return (
            <div
              key={source.name}
              className={cn(
                "rounded-xl border px-3 py-2.5 transition-colors",
                state === "ok" && "border-emerald-500/25 bg-emerald-500/[0.06]",
                state === "deferred" && "border-amber-500/30 bg-amber-500/[0.07]",
                state === "error" && "border-red-500/30 bg-red-500/[0.07]",
                state === "idle" && "border-border/70 bg-muted/20 opacity-75",
              )}
              title={source.errors.length > 0 ? source.errors.join("\n") : undefined}
            >
              <div className="flex items-center justify-between gap-2">
                <div className="flex min-w-0 items-center gap-1.5">
                  {icon ? (
                    <AppBrandIcon icon={icon} name={source.name} size={16} />
                  ) : (
                    <Database className="h-4 w-4 shrink-0 text-muted-foreground" />
                  )}
                  <span className="truncate text-xs font-semibold">{source.name}</span>
                </div>
                {state === "ok" && <CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />}
                {state === "deferred" && <Clock3 className="h-3.5 w-3.5 text-amber-500" />}
                {state === "error" && <AlertTriangle className="h-3.5 w-3.5 text-red-500" />}
                {state === "idle" && <CircleDashed className="h-3.5 w-3.5 text-muted-foreground" />}
              </div>
              <div className="mt-2 text-[0.68rem] text-muted-foreground">
                {source.detected ? t("sources.detected") : t("sources.notDetected")}
              </div>
              <div className="mt-1 flex flex-wrap gap-x-2 gap-y-0.5 text-[0.68rem] tabular-nums text-muted-foreground">
                <span>{t("sources.imported", { count: source.imported })}</span>
                {source.deferredFiles > 0 && (
                  <span className="text-amber-600 dark:text-amber-400">
                    {t("sources.deferred", { count: source.deferredFiles })}
                  </span>
                )}
                {source.errors.length > 0 && (
                  <span className="text-red-600 dark:text-red-400">
                    {t("sources.errors", { count: source.errors.length })}
                  </span>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
