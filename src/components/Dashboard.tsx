import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  BarChart3,
  LayoutGrid,
  RefreshCw,
} from "lucide-react";
import type { DateRange, LogFilters, RangePreset } from "../types/usage";
import { makeRange } from "../types/usage";
import { useUsageEventBridge } from "../lib/eventBridge";
import {
  useDailyTrends,
  useModelStats,
  useProviderStats,
  useSyncSessionLogs,
  useUsageSummary,
} from "../lib/hooks";
import { UsageHero } from "./UsageHero";
import { UsageTrendChart } from "./UsageTrendChart";
import { ModelStatsTable } from "./ModelStatsTable";
import { ProviderStatsTable } from "./ProviderStatsTable";
import { RequestLogTable } from "./RequestLogTable";
import { AppBrandIcon, type AppBrandIconName } from "./AppBrandIcon";

const PRESETS: { label: string; value: RangePreset }[] = [
  { label: "今天", value: "today" },
  { label: "24h", value: "1d" },
  { label: "7天", value: "7d" },
  { label: "14天", value: "14d" },
  { label: "30天", value: "30d" },
  { label: "90天", value: "90d" },
  { label: "全部", value: "all" },
];

const APP_FILTERS = [
  { value: "all", label: "全部应用", icon: null },
  { value: "claude", label: "Claude Code", icon: "claude" },
  { value: "codex", label: "Codex", icon: "openai" },
  { value: "gemini", label: "Gemini", icon: "gemini" },
  { value: "grokbuild", label: "Grok Build", icon: "grok" },
  { value: "opencode", label: "OpenCode", icon: "opencode" },
] as const;

const REFRESH_INTERVALS = [
  { label: "关闭", value: 0 },
  { label: "5s", value: 5_000 },
  { label: "30s", value: 30_000 },
  { label: "60s", value: 60_000 },
];

type StatsTab = "providers" | "models";
type AppFilter = (typeof APP_FILTERS)[number]["value"];

export function Dashboard() {
  useUsageEventBridge();
  const queryClient = useQueryClient();

  // 与 CC Switch 一致，默认展示今天；更长范围由顶部预设显式选择。
  const [range, setRange] = useState<DateRange>(() => makeRange("today"));
  const [statsTab, setStatsTab] = useState<StatsTab>("models");
  const [appType, setAppType] = useState<AppFilter>("all");
  const [providerName, setProviderName] = useState("");
  const [model, setModel] = useState("");
  const [refreshIntervalMs, setRefreshIntervalMs] = useState(5_000);
  const sync = useSyncSessionLogs();

  const params = useMemo(
    () => ({
      startDate: range.start,
      endDate: range.end,
      appType: appType === "all" ? null : appType,
      providerName: providerName || null,
      model: model || null,
    }),
    [appType, model, providerName, range],
  );

  const optionParams = useMemo(
    () => ({
      startDate: range.start,
      endDate: range.end,
      appType: appType === "all" ? null : appType,
      providerName: null,
      model: null,
    }),
    [appType, range],
  );

  const modelOptionParams = useMemo(
    () => ({ ...optionParams, providerName: providerName || null }),
    [optionParams, providerName],
  );

  const summary = useUsageSummary(params, refreshIntervalMs);
  const trends = useDailyTrends(params, refreshIntervalMs);
  const providerStats = useProviderStats(params, refreshIntervalMs);
  const modelStats = useModelStats(params, refreshIntervalMs);
  const providerOptionsQuery = useProviderStats(optionParams, refreshIntervalMs);
  const modelOptionsQuery = useModelStats(modelOptionParams, refreshIntervalMs);
  const providerOptions = useMemo(
    () => uniqueNames(providerOptionsQuery.data?.map((row) => row.providerName), providerName),
    [providerName, providerOptionsQuery.data],
  );
  const modelOptions = useMemo(
    () => uniqueNames(modelOptionsQuery.data?.map((row) => row.model), model),
    [model, modelOptionsQuery.data],
  );
  const loadError =
    summary.error ??
    trends.error ??
    providerStats.error ??
    modelStats.error ??
    providerOptionsQuery.error ??
    modelOptionsQuery.error ??
    sync.error;

  const changeAppType = (next: AppFilter) => {
    setAppType(next);
    setProviderName("");
    setModel("");
  };

  const changeProviderName = (next: string) => {
    setProviderName(next);
    setModel("");
  };

  const handleSync = async () => {
    try {
      await sync.refetch();
    } finally {
      // 手动同步即使没有新增文件，也要重新查询一次：首次启动时统计查询
      // 可能早于后台同步完成而进入 error 状态。
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["usage"] }),
      ]);
    }
  };

  const filters: LogFilters = useMemo(
    () => ({
      appType: appType === "all" ? null : appType,
      providerName: providerName || null,
      model: model || null,
      startDate: range.start,
      endDate: range.end,
    }),
    [appType, model, providerName, range],
  );

  return (
    <div className="min-h-screen bg-background text-foreground">
      <div className="mx-auto max-w-6xl space-y-4 p-4 md:p-6">
        {/* 顶部工具栏 */}
        <header className="flex flex-col gap-3 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <h1 className="text-xl font-semibold tracking-tight">使用统计</h1>
            <p className="text-xs text-muted-foreground">查看 AI 模型的使用情况和成本统计</p>
          </div>
          <div className="flex flex-wrap items-center justify-end gap-1.5">
            <div className="flex items-center rounded-lg border border-border bg-card p-0.5">
              {APP_FILTERS.map(({ value, label, icon }) => (
                <button
                  key={value}
                  type="button"
                  title={label}
                  aria-label={label}
                  aria-pressed={appType === value}
                  onClick={() => changeAppType(value)}
                  className={`flex h-8 items-center justify-center rounded-md px-2.5 transition-colors ${
                    appType === value
                      ? "bg-primary text-primary-foreground shadow-sm"
                      : "text-muted-foreground hover:bg-muted hover:text-foreground"
                  }`}
                >
                  {icon ? (
                    <AppBrandIcon icon={icon as AppBrandIconName} name={label} size={16} />
                  ) : (
                    <LayoutGrid className="h-4 w-4" />
                  )}
                </button>
              ))}
            </div>

            <select
              aria-label="按来源筛选"
              value={providerName}
              onChange={(event) => changeProviderName(event.target.value)}
              className="h-9 max-w-[150px] rounded-md border border-border bg-card px-2.5 text-xs outline-none focus:border-primary"
            >
              <option value="">全部来源</option>
              {providerOptions.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>

            <select
              aria-label="按模型筛选"
              value={model}
              onChange={(event) => setModel(event.target.value)}
              className="h-9 max-w-[150px] rounded-md border border-border bg-card px-2.5 text-xs outline-none focus:border-primary"
            >
              <option value="">全部模型</option>
              {modelOptions.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>

            <select
              aria-label="自动刷新间隔"
              value={refreshIntervalMs}
              onChange={(event) => setRefreshIntervalMs(Number(event.target.value))}
              className="h-9 w-[78px] rounded-md border border-border bg-card px-2.5 text-xs outline-none focus:border-primary"
            >
              {REFRESH_INTERVALS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>

            <div className="flex whitespace-nowrap rounded-md border border-border bg-card p-0.5">
              {PRESETS.map((p) => (
                <button
                  key={p.value}
                  onClick={() => setRange(makeRange(p.value))}
                    className={`rounded px-2 py-1 text-xs transition-colors ${
                    range.preset === p.value
                      ? "bg-primary text-primary-foreground"
                      : "text-muted-foreground hover:bg-muted"
                  }`}
                >
                  {p.label}
                </button>
              ))}
            </div>

            <button
              onClick={() => void handleSync()}
              disabled={sync.isFetching}
              className="flex items-center gap-1 rounded-md border border-border bg-card px-2.5 py-1 text-xs hover:bg-muted disabled:opacity-50"
              title="立即同步本地会话日志"
            >
              <RefreshCw
                className={`h-3.5 w-3.5 ${sync.isFetching ? "animate-spin" : ""}`}
              />
              同步
            </button>
          </div>
        </header>

        {loadError && (
          <div
            role="alert"
            className="rounded-md border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-500"
          >
            使用统计加载失败：{getErrorMessage(loadError)}。请点击“同步”重试。
          </div>
        )}

        {/* 同步状态 */}
        {sync.data && (
          <div className="rounded-md border border-border/50 bg-muted/30 px-3 py-1.5 text-xs text-muted-foreground">
            {sync.data.imported > 0 ? (
              <span>
                本次同步: 导入 {sync.data.imported} 条 / 跳过{" "}
                {sync.data.skipped} 条 / 扫描 {sync.data.filesScanned} 个文件
              </span>
            ) : (
              <span>已是最新 (扫描 {sync.data.filesScanned} 个文件)</span>
            )}
            {sync.data.errors.length > 0 && (
              <span className="text-red-500">
                {" "}
                · {sync.data.errors.length} 个错误
              </span>
            )}
          </div>
        )}

        {/* 概览卡片 */}
        <UsageHero
          summary={summary.data}
          isLoading={summary.isLoading}
          appType={appType === "all" ? null : appType}
        />

        {/* 趋势图 */}
        <UsageTrendChart
          data={trends.data}
          isLoading={trends.isLoading}
          startDate={range.start}
          endDate={range.end}
          rangeLabel={
            range.preset === "today"
              ? "当天"
              : PRESETS.find((preset) => preset.value === range.preset)?.label ?? "自定义"
          }
        />

        {/* 维度统计：参考 CC Switch，按供应商/模型切换查看 */}
        <section className="space-y-3">
          <div
            role="tablist"
            aria-label="统计维度"
            className="inline-flex items-center gap-1 rounded-lg border border-border bg-muted/40 p-1"
          >
            <button
              type="button"
              role="tab"
              aria-selected={statsTab === "providers"}
              onClick={() => setStatsTab("providers")}
              className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                statsTab === "providers"
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground"
              }`}
            >
              <Activity className="h-3.5 w-3.5" />
              供应商统计
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={statsTab === "models"}
              onClick={() => setStatsTab("models")}
              className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                statsTab === "models"
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground"
              }`}
            >
              <BarChart3 className="h-3.5 w-3.5" />
              模型统计
            </button>
          </div>

          {statsTab === "providers" ? (
            <ProviderStatsTable
              data={providerStats.data}
              isLoading={providerStats.isLoading}
            />
          ) : (
            <ModelStatsTable data={modelStats.data} isLoading={modelStats.isLoading} />
          )}
        </section>

        {/* 请求日志 */}
        <RequestLogTable filters={filters} refetchMs={refreshIntervalMs} />
      </div>
    </div>
  );
}

function uniqueNames(values: Array<string | undefined> | undefined, selected: string): string[] {
  const names = new Set((values ?? []).filter((value): value is string => Boolean(value)));
  if (selected) names.add(selected);
  return Array.from(names);
}

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
