import { type ChangeEvent, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  Activity,
  BarChart3,
  CircleDollarSign,
  Download,
  LayoutGrid,
  RefreshCw,
  SlidersHorizontal,
  Upload,
} from "lucide-react";
import type { DateRange, LogFilters, RangePreset } from "../types/usage";
import { makeRange } from "../types/usage";
import { useUsageEventBridge } from "../lib/eventBridge";
import {
  useDailyTrends,
  useDevices,
  useModelPricing,
  useModelStats,
  useProviderStats,
  useRefreshModelPricing,
  useSyncSessionLogs,
  useUsageSummary,
  usageKeys,
} from "../lib/hooks";
import { exportUsageData, importUsageData } from "../lib/api";
import { UsageHero } from "./UsageHero";
import { UsageInsights } from "./UsageInsights";
import { CodexRadarPanel } from "./CodexRadarPanel";
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
  { value: "all", label: "全部应用", icon: null, providerName: "" },
  { value: "claude", label: "Claude Code", icon: "claude", providerName: "Claude (Session)" },
  { value: "codex", label: "Codex", icon: "openai", providerName: "Codex (Session)" },
  { value: "gemini", label: "Gemini", icon: "gemini", providerName: "Gemini (Session)" },
  {
    value: "grokbuild",
    label: "Grok Build",
    icon: "grok",
    providerName: "Grok Build (Session)",
  },
  { value: "opencode", label: "OpenCode", icon: "opencode", providerName: "OpenCode (Session)" },
  { value: "zcode", label: "ZCode", icon: "zcode", providerName: "ZCode (Session)" },
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
  const [deviceId, setDeviceId] = useState("");
  const [refreshIntervalMs, setRefreshIntervalMs] = useState(5_000);
  const [isTransferring, setIsTransferring] = useState(false);
  const importInputRef = useRef<HTMLInputElement>(null);
  const priceAutoRefreshAttemptedRef = useRef(false);
  const sync = useSyncSessionLogs();
  const pricing = useModelPricing();
  const refreshPricing = useRefreshModelPricing();
  const pricingCacheSummary = useMemo(() => {
    const rows = pricing.data ?? [];
    const liveModels = rows.filter((row) => row.priceSource === "OpenRouter live").length;
    const latestFetchedAt = rows.reduce(
      (latest, row) => Math.max(latest, row.priceFetchedAt ?? 0),
      0,
    );
    return {
      modelCount: rows.length,
      liveModels,
      fallbackModels: Math.max(0, rows.length - liveModels),
      latestFetchedAt,
    };
  }, [pricing.data]);
  const devices = useDevices();
  const refreshUsageQueries = useCallback(
    () =>
      queryClient.refetchQueries({
        queryKey: usageKeys.all,
        type: "active",
        // 不要重新触发同步查询本身，否则同步完成后的重查会形成循环。
        predicate: (query) => query.queryKey[1] !== "sync",
      }),
    [queryClient],
  );

  useEffect(() => {
    // 首次启动时同步和统计查询是并行的。同步完成后补一次重查，避免统计
    // 只读到同步过程中的部分日志；手动同步也复用这条链路。
    if (!sync.isSuccess || sync.isFetching || sync.dataUpdatedAt === 0) return;
    void refreshUsageQueries();
  }, [refreshUsageQueries, sync.dataUpdatedAt, sync.isFetching, sync.isSuccess]);

  const params = useMemo(
    () => ({
      startDate: range.start,
      endDate: range.end,
      appType: appType === "all" ? null : appType,
      providerName: providerName || null,
      model: model || null,
      deviceId: deviceId || null,
    }),
    [appType, deviceId, model, providerName, range],
  );

  const optionParams = useMemo(
    () => ({
      startDate: range.start,
      endDate: range.end,
      appType: appType === "all" ? null : appType,
      providerName: null,
      model: null,
      deviceId: deviceId || null,
    }),
    [appType, deviceId, range],
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
  const isRefreshing =
    summary.isFetching ||
    trends.isFetching ||
    providerStats.isFetching ||
    modelStats.isFetching ||
    providerOptionsQuery.isFetching ||
    modelOptionsQuery.isFetching;
  const lastUpdatedAt = Math.max(
    summary.dataUpdatedAt,
    trends.dataUpdatedAt,
    providerStats.dataUpdatedAt,
    modelStats.dataUpdatedAt,
    providerOptionsQuery.dataUpdatedAt,
    modelOptionsQuery.dataUpdatedAt,
  );
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
    devices.error ??
    sync.error;

  const selectAppFilter = (next: AppFilter, selectedProviderName?: string) => {
    setAppType(next);
    setProviderName(
      selectedProviderName ?? APP_FILTERS.find((filter) => filter.value === next)?.providerName ?? "",
    );
    setModel("");
    // ZCode 本机记录目前主要是历史数据，避免“今天”窗口切换后看起来像没有接入。
    if (next === "zcode" && range.preset === "today") {
      setRange(makeRange("all"));
    }
  };

  const changeAppType = (next: AppFilter) => {
    selectAppFilter(next);
  };

  const changeProviderName = (next: string) => {
    selectAppFilter(
      APP_FILTERS.find((filter) => filter.providerName === next)?.value ?? "all",
      next,
    );
  };

  const handleSync = async () => {
    await sync.refetch();
  };

  const handleRefreshPricing = async () => {
    priceAutoRefreshAttemptedRef.current = true;
    try {
      const result = await refreshPricing.mutateAsync();
      await queryClient.invalidateQueries({ queryKey: usageKeys.pricing });
      await refreshUsageQueries();
      toast.success(
        `在线价格已更新 ${result.updatedModels} 个模型，新增 ${result.addedModels} 个，回填 ${result.recostedRecords} 条记录`,
      );
    } catch (error) {
      toast.error(`在线价格更新失败：${getErrorMessage(error)}，继续使用本地价格缓存`);
    }
  };

  useEffect(() => {
    if (!pricing.isSuccess || priceAutoRefreshAttemptedRef.current) return;
    const fetchedAt = pricingCacheSummary.latestFetchedAt;
    const isStale = !fetchedAt || Date.now() / 1000 - fetchedAt > 6 * 60 * 60;
    priceAutoRefreshAttemptedRef.current = true;
    if (!isStale) return;

    // 启动时静默尝试一次；失败时保留内置价，不打断本地统计。
    void refreshPricing.mutateAsync().then(async () => {
      await queryClient.invalidateQueries({ queryKey: usageKeys.pricing });
      await refreshUsageQueries();
    }).catch(() => undefined);
  }, [pricing.isSuccess, pricingCacheSummary.latestFetchedAt, queryClient, refreshPricing, refreshUsageQueries]);

  const handleExport = async () => {
    setIsTransferring(true);
    try {
      const payload = await exportUsageData();
      const url = URL.createObjectURL(
        new Blob([payload.contents], { type: "application/json;charset=utf-8" }),
      );
      const link = document.createElement("a");
      link.href = url;
      link.download = payload.fileName;
      document.body.appendChild(link);
      link.click();
      link.remove();
      window.setTimeout(() => URL.revokeObjectURL(url), 1_000);
      toast.success(`已导出 ${payload.recordCount.toLocaleString("zh-CN")} 条记录`);
    } catch (error) {
      toast.error(`导出失败：${getErrorMessage(error)}`);
    } finally {
      setIsTransferring(false);
    }
  };

  const handleImport = async (event: ChangeEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    if (file.size > 250 * 1024 * 1024) {
      toast.error("导入文件不能超过 250MB");
      return;
    }

    setIsTransferring(true);
    try {
      const result = await importUsageData(await file.text());
      await queryClient.invalidateQueries({ queryKey: usageKeys.all });
      toast.success(
        `合并完成：新增 ${result.imported.toLocaleString("zh-CN")} 条，跳过 ${result.skipped.toLocaleString("zh-CN")} 条重复记录`,
      );
    } catch (error) {
      toast.error(`导入失败：${getErrorMessage(error)}`);
    } finally {
      setIsTransferring(false);
    }
  };

  const filters: LogFilters = useMemo(
    () => ({
      appType: appType === "all" ? null : appType,
      providerName: providerName || null,
      model: model || null,
      deviceId: deviceId || null,
      startDate: range.start,
      endDate: range.end,
    }),
    [appType, deviceId, model, providerName, range],
  );

  return (
    <div className="app-shell text-foreground">
      <div className="app-content mx-auto max-w-7xl space-y-5 px-4 py-5 md:px-6 md:py-7 lg:px-8">
        {/* 顶部工具栏 */}
        <header className="dashboard-header flex flex-col gap-5 lg:flex-row lg:items-center lg:justify-between">
          <div className="flex items-center gap-3.5">
            <div className="brand-orb" aria-hidden="true">
              <Activity className="relative z-10 h-6 w-6" />
            </div>
            <div>
              <div className="dashboard-kicker">
                <span className="status-dot" />
                LIVE / USAGE PULSE
              </div>
              <h1 className="dashboard-title">
                使用统计 <span>CONTROL CENTER</span>
              </h1>
              <p className="dashboard-subtitle">查看 AI 模型的使用情况和成本统计</p>
            </div>
          </div>
          <div className="system-pill self-start lg:self-auto">
            <span className={`system-dot ${isRefreshing ? "is-busy" : ""}`} />
            {isRefreshing ? "实时同步中" : "数据已就绪"}
          </div>
        </header>

        <section className="control-panel">
          <div className="control-panel-header">
            <div className="control-caption">
              <div className="control-caption-icon" aria-hidden="true">
                <SlidersHorizontal className="h-4 w-4" />
              </div>
              <div>
                <div className="control-caption-title">FILTER MATRIX</div>
                <div className="control-caption-subtitle">选择设备、应用、来源和时间窗口</div>
              </div>
            </div>
            <span className="control-panel-hint">MULTI-DEVICE / LOCAL</span>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <select
              aria-label="按设备筛选"
              value={deviceId}
              onChange={(event) => setDeviceId(event.target.value)}
              className="control-select device-select"
            >
              <option value="">全部设备（汇总）</option>
              {(devices.data ?? []).map((device) => (
                <option key={device.id} value={device.id}>
                  {device.isLocal
                    ? `本机 · ${device.name}`
                    : `${device.name} · ${device.requestCount.toLocaleString("zh-CN")} 条`}
                </option>
              ))}
            </select>

            <div className="app-filter-group" role="group" aria-label="按应用筛选">
              {APP_FILTERS.map(({ value, label, icon }) => (
                <button
                  key={value}
                  type="button"
                  title={label}
                  aria-label={label}
                  aria-pressed={appType === value}
                  onClick={() => changeAppType(value)}
                  className={`app-filter-button flex h-8 items-center justify-center rounded-md px-2.5 ${
                    appType === value ? "is-active" : ""
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
              className="control-select"
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
              className="control-select"
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
              title={
                refreshIntervalMs > 0
                  ? `每 ${refreshIntervalMs / 1000} 秒自动刷新统计数据`
                  : "已关闭自动刷新"
              }
              className="control-select w-[78px] min-w-0"
            >
              {REFRESH_INTERVALS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>

            <div className="range-group flex-wrap whitespace-nowrap">
              {PRESETS.map((p) => (
                <button
                  key={p.value}
                  onClick={() => setRange(makeRange(p.value))}
                  className={`range-button rounded px-2 py-1 text-xs ${
                    range.preset === p.value ? "is-active" : ""
                  }`}
                >
                  {p.label}
                </button>
              ))}
            </div>

            <div className="transfer-button-group" role="group" aria-label="多设备数据迁移">
              <button
                type="button"
                onClick={() => void handleExport()}
                disabled={isTransferring}
                className="transfer-button"
                title="导出全部设备记录，复制到其他电脑后导入"
              >
                <Download className="h-3.5 w-3.5" />
                导出
              </button>
              <button
                type="button"
                onClick={() => importInputRef.current?.click()}
                disabled={isTransferring}
                className="transfer-button"
                title="导入其他电脑导出的 JSON，自动跳过重复记录"
              >
                <Upload className="h-3.5 w-3.5" />
                导入
              </button>
              <input
                ref={importInputRef}
                type="file"
                accept="application/json,.json"
                onChange={(event) => void handleImport(event)}
                hidden
                tabIndex={-1}
              />
            </div>

            <button
              onClick={() => void handleSync()}
              disabled={sync.isFetching}
              className="sync-button"
              title="立即同步本地会话日志"
            >
              <RefreshCw
                className={`h-3.5 w-3.5 ${sync.isFetching ? "animate-spin" : ""}`}
              />
              {sync.isFetching ? "同步中..." : "同步"}
            </button>

            <button
              type="button"
              onClick={() => void handleRefreshPricing()}
              disabled={refreshPricing.isPending}
              className="sync-button"
              title="从公开在线目录更新 API Token 价格，不改变用量取数"
            >
              <CircleDollarSign
                className={`h-3.5 w-3.5 ${refreshPricing.isPending ? "animate-spin" : ""}`}
              />
              {refreshPricing.isPending ? "价格中..." : "更新价格"}
            </button>
          </div>
        </section>

        {loadError && (
          <div
            role="alert"
            className="alert-panel text-xs"
          >
            使用统计加载失败：{getErrorMessage(loadError)}。请点击“同步”重试。
          </div>
        )}

        {/* 同步与自动刷新状态 */}
        {(sync.isFetching || sync.data || isRefreshing || lastUpdatedAt > 0 || pricing.data) && (
          <div className="status-panel flex-wrap">
            <div className="flex items-center gap-2">
              {sync.isFetching ? (
                <span className="text-primary">正在同步本地会话日志...</span>
              ) : sync.data ? (
                <>
                  {sync.data.imported > 0 ? (
                    <span>
                      本次同步: 导入 {sync.data.imported} 条 / 跳过{" "}
                      {sync.data.skipped} 条 / 扫描 {sync.data.filesScanned} 个文件
                    </span>
                  ) : (
                    <span>已是最新 (扫描 {sync.data.filesScanned} 个文件)</span>
                  )}
                  {sync.data.errors.length > 0 && (
                    <span className="text-red-500">· {sync.data.errors.length} 个错误</span>
                  )}
                </>
              ) : null}
              {isRefreshing && !sync.isFetching && (
                <span className="text-primary">正在刷新统计...</span>
              )}
              {refreshPricing.isPending && (
                <span className="text-primary">正在联网更新价格...</span>
              )}
            </div>
            <div className="flex items-center gap-2">
              {refreshIntervalMs > 0 && <span>每 {refreshIntervalMs / 1000}s 自动刷新</span>}
              {lastUpdatedAt > 0 && <span>最近更新 {formatTime(lastUpdatedAt)}</span>}
            </div>
            {pricingCacheSummary.modelCount > 0 && (
              <div
                className="flex items-center gap-1.5 text-xs"
                title="在线价格成功后会按模型写入本地缓存；成本计算只读取本地缓存"
              >
                <CircleDollarSign className="h-3.5 w-3.5 text-primary" />
                <span>
                  价格缓存：在线 {pricingCacheSummary.liveModels} 个 / 回退 {pricingCacheSummary.fallbackModels} 个
                </span>
                <span>·</span>
                <span>
                  {pricingCacheSummary.latestFetchedAt
                    ? `${formatTime(pricingCacheSummary.latestFetchedAt * 1000)} 更新`
                    : "尚未联网，使用本地回退价"}
                </span>
              </div>
            )}
          </div>
        )}

        {/* 第三方社区雷达独立联网，优先展示；失败时不影响本地统计。 */}
        <CodexRadarPanel />

        {/* 概览卡片 */}
        <UsageHero
          summary={summary.data}
          isLoading={summary.isLoading}
          appType={appType === "all" ? null : appType}
        />

        {/* 从当前已加载数据派生的分析洞察，不增加新的取数链路 */}
        <UsageInsights
          summary={summary.data}
          trends={trends.data}
          providers={providerStats.data}
          models={modelStats.data}
          isLoading={
            summary.isLoading ||
            trends.isLoading ||
            providerStats.isLoading ||
            modelStats.isLoading
          }
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
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div className="panel-kicker">BREAKDOWN / DIMENSIONS</div>
              <div className="mt-1 text-xs text-muted-foreground">按供应商或模型查看细分</div>
            </div>
            <div
              role="tablist"
              aria-label="统计维度"
              className="tab-switcher"
            >
            <button
              type="button"
              role="tab"
              aria-selected={statsTab === "providers"}
              onClick={() => setStatsTab("providers")}
              className={`tab-button flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium ${
                statsTab === "providers" ? "is-active" : ""
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
              className={`tab-button flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs font-medium ${
                statsTab === "models" ? "is-active" : ""
              }`}
            >
              <BarChart3 className="h-3.5 w-3.5" />
              模型统计
            </button>
            </div>
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

function formatTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function getErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
