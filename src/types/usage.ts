// 与后端 Rust 类型对齐（serde camelCase）

export interface UsageSummary {
  totalRequests: number;
  totalCost: string;
  unpricedRequests: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  realTotalTokens: number;
  successRate: number;
  cacheHitRate: number;
}

export interface RuntimeStats {
  sourceCount: number;
  sessionCount: number;
  turns: number;
  steps: number;
  llmMs: number;
  toolMs: number;
  ttftMs: number;
  ttftSteps: number;
  decodeMs: number;
  decodeTokens: number;
  averageTtftMs: number | null;
  decodeTokensPerSecond: number | null;
}

export interface DailyStats {
  date: string;
  requestCount: number;
  totalCost: string;
  totalTokens: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
}

export interface ProviderStats {
  providerId: string;
  providerName: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  successRate: number;
  avgLatencyMs: number | null;
}

export interface ModelStats {
  model: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  avgCostPerRequest: string;
}

export interface UsageBreakdownStats {
  key: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  avgCostPerRequest: string;
  unpricedRequests: number;
  sourceName: string;
  appType: string;
  latestAt: number;
}

export interface RequestLogDetail {
  requestId: string;
  providerId: string;
  providerName: string;
  appType: string;
  model: string;
  requestModel: string | null;
  pricingModel: string | null;
  inputTokens: number;
  freshInputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  totalCostUsd: string;
  inputCostUsd: string;
  outputCostUsd: string;
  cacheReadCostUsd: string;
  cacheCreationCostUsd: string;
  latencyMs: number;
  statusCode: number;
  errorMessage: string | null;
  sessionId: string | null;
  isStreaming: boolean;
  costMultiplier: string;
  createdAt: number;
  dataSource: string;
  project: string;
}

export interface LogFilters {
  appType?: string | null;
  providerName?: string | null;
  model?: string | null;
  deviceId?: string | null;
  statusCode?: number | null;
  startDate?: number | null;
  endDate?: number | null;
}

export interface PaginatedLogs {
  data: RequestLogDetail[];
  total: number;
  page: number;
  pageSize: number;
}

export interface ModelPricingInfo {
  modelId: string;
  displayName: string;
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheCreationCostPerMillion: string;
  priceSource: string;
  priceFetchedAt: number | null;
}

export interface PricingRefreshResult {
  source: string;
  fetchedAt: number;
  catalogModels: number;
  updatedModels: number;
  addedModels: number;
  recostedRecords: number;
}

export interface SessionSyncResult {
  imported: number;
  skipped: number;
  filesScanned: number;
  suspectedDuplicates: number;
  deferredFiles: number;
  errors: string[];
  sourceStatuses: SyncSourceStatus[];
}

export interface SyncSourceStatus {
  name: string;
  detected: boolean;
  imported: number;
  skipped: number;
  filesScanned: number;
  deferredFiles: number;
  errors: string[];
}

export interface DeviceInfo {
  id: string;
  name: string;
  isLocal: boolean;
  requestCount: number;
  lastSeen: number | null;
}

export interface UsageExportPayload {
  fileName: string;
  contents: string;
  recordCount: number;
}

export interface UsageImportResult {
  imported: number;
  skipped: number;
  deviceCount: number;
}

export interface CodexRadarSnapshot {
  iq: RadarIqSnapshot | null;
  quota: RadarQuotaSnapshot | null;
  fetchedAt: number;
  usedFallback: boolean;
  warnings: string[];
}

export interface RadarIqSnapshot {
  sourceUpdatedAt: string;
  runs24hTotal: number;
  runsTotal: number;
  benchmarkId: string;
  scoreLabel: string;
  points: RadarIqPoint[];
}

export interface RadarIqPoint {
  model: string;
  effort: string;
  iq: number;
  averagePriceUsd: number | null;
  averageMinutes: number | null;
  total: number;
  runs24h: number;
  runsTotal: number;
  sourceUpdatedAt: string | null;
}

export interface RadarQuotaSnapshot {
  sourceUpdatedAt: string;
  tiers: RadarQuotaTier[];
  history: RadarQuotaHistoryPoint[];
}

export interface RadarQuotaTier {
  plan: string;
  weeklyUsd: number;
  source: string;
}

export interface RadarQuotaHistoryPoint {
  label: string;
  weeklyUsd: number;
}

// 时间范围预设
export type RangePreset = "today" | "1d" | "7d" | "14d" | "30d" | "90d" | "all" | "custom";

export interface DateRange {
  preset: RangePreset;
  start: number;
  end: number;
}

export function makeRange(preset: RangePreset, customStart?: number, customEnd?: number): DateRange {
  const end = Math.floor(Date.now() / 1000);
  const day = 24 * 3600;

  const startOfLocalDay = (timestamp: number) => {
    const date = new Date(timestamp * 1000);
    date.setHours(0, 0, 0, 0);
    return Math.floor(date.getTime() / 1000);
  };

  switch (preset) {
    case "today": {
      // 本地午夜
      const d = new Date();
      d.setHours(0, 0, 0, 0);
      return { preset, start: Math.floor(d.getTime() / 1000), end };
    }
    case "1d":
      return { preset, start: end - day, end };
    case "7d":
      return { preset, start: startOfLocalDay(end - 6 * day), end };
    case "14d":
      return { preset, start: startOfLocalDay(end - 13 * day), end };
    case "30d":
      return { preset, start: startOfLocalDay(end - 29 * day), end };
    case "90d":
      return { preset, start: startOfLocalDay(end - 89 * day), end };
    case "all":
      // 覆盖全部历史：起始设为 2000-01-01，确保所有历史数据都被查到
      return { preset, start: 946684800, end };
    case "custom":
      return {
        preset,
        start: customStart ?? end - 7 * day,
        end: customEnd ?? end,
      };
  }
}
