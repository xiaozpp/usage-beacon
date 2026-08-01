import { invoke } from "@tauri-apps/api/core";
import type {
  DailyStats,
  LogFilters,
  ModelPricingInfo,
  ModelStats,
  PaginatedLogs,
  ProviderStats,
  RequestLogDetail,
  SessionSyncResult,
  UsageSummary,
} from "../types/usage";

export interface UsageQueryParams {
  startDate: number;
  endDate: number;
  appType?: string | null;
  providerName?: string | null;
  model?: string | null;
  [key: string]: number | string | null | undefined;
}

export async function syncSessionLogs(): Promise<SessionSyncResult> {
  return invoke<SessionSyncResult>("sync_claude_logs");
}

export async function fetchUsageSummary(params: UsageQueryParams): Promise<UsageSummary> {
  return invoke<UsageSummary>("fetch_usage_summary", params);
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
