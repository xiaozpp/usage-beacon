import { invoke } from "@tauri-apps/api/core";
import type {
  DailyStats,
  CodexRadarSnapshot,
  RuntimeStats,
  DeviceInfo,
  LogFilters,
  ModelPricingInfo,
  ModelStats,
  PaginatedLogs,
  PricingRefreshResult,
  ProviderStats,
  RequestLogDetail,
  SessionSyncResult,
  UsageBreakdownStats,
  UsageSummary,
  UsageExportPayload,
  UsageImportResult,
} from "../types/usage";

export interface UsageQueryParams {
  startDate: number;
  endDate: number;
  appType?: string | null;
  providerName?: string | null;
  model?: string | null;
  deviceId?: string | null;
  [key: string]: number | string | null | undefined;
}

export async function syncSessionLogs(): Promise<SessionSyncResult> {
  return invoke<SessionSyncResult>("sync_session_logs");
}

export async function fetchUsageSummary(params: UsageQueryParams): Promise<UsageSummary> {
  return invoke<UsageSummary>("fetch_usage_summary", params);
}

export async function fetchRuntimeStats(
  params: UsageQueryParams,
): Promise<RuntimeStats> {
  return invoke<RuntimeStats>("fetch_runtime_stats", params);
}

export async function fetchDailyTrends(params: UsageQueryParams): Promise<DailyStats[]> {
  return invoke<DailyStats[]>("fetch_daily_trends", params);
}

export async function fetchProviderStats(params: UsageQueryParams): Promise<ProviderStats[]> {
  return invoke<ProviderStats[]>("fetch_provider_stats", params);
}

export async function fetchModelStats(params: UsageQueryParams): Promise<ModelStats[]> {
  return invoke<ModelStats[]>("fetch_model_stats", params);
}

export async function fetchProjectStats(params: UsageQueryParams): Promise<UsageBreakdownStats[]> {
  return invoke<UsageBreakdownStats[]>("fetch_project_stats", params);
}

export async function fetchSessionStats(params: UsageQueryParams): Promise<UsageBreakdownStats[]> {
  return invoke<UsageBreakdownStats[]>("fetch_session_stats", params);
}

export async function fetchRequestLogs(
  filters: LogFilters,
  page: number,
  pageSize: number,
): Promise<PaginatedLogs> {
  return invoke<PaginatedLogs>("fetch_request_logs", {
    filters,
    page,
    pageSize,
  });
}

export async function fetchRequestDetail(requestId: string): Promise<RequestLogDetail | null> {
  return invoke<RequestLogDetail | null>("fetch_request_detail", { requestId });
}

export async function fetchModelPricing(): Promise<ModelPricingInfo[]> {
  return invoke<ModelPricingInfo[]>("fetch_model_pricing");
}

export async function refreshModelPricing(): Promise<PricingRefreshResult> {
  return invoke<PricingRefreshResult>("refresh_model_pricing");
}

export async function fetchDevices(): Promise<DeviceInfo[]> {
  return invoke<DeviceInfo[]>("fetch_devices");
}

export async function fetchCodexRadar(): Promise<CodexRadarSnapshot> {
  return invoke<CodexRadarSnapshot>("fetch_codex_radar");
}

export async function exportUsageData(): Promise<UsageExportPayload> {
  return invoke<UsageExportPayload>("export_usage_data");
}

export async function importUsageData(contents: string): Promise<UsageImportResult> {
  return invoke<UsageImportResult>("import_usage_data", { contents });
}
