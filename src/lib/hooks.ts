import { useMutation, useQuery } from "@tanstack/react-query";
import { fetchCodexRadar, fetchDevices, fetchModelPricing, fetchUsageSummary, fetchDailyTrends, fetchModelStats, fetchProviderStats, fetchRequestLogs, fetchRequestDetail, refreshModelPricing, syncSessionLogs, type UsageQueryParams } from "./api";
import type { LogFilters } from "../types/usage";

export const usageKeys = {
  all: ["usage"] as const,
  summary: (params: UsageQueryParams) => ["usage", "summary", params] as const,
  trends: (params: UsageQueryParams) => ["usage", "trends", params] as const,
  providerStats: (params: UsageQueryParams) => ["usage", "providers", params] as const,
  modelStats: (params: UsageQueryParams) => ["usage", "models", params] as const,
  logs: (filters: LogFilters, page: number, pageSize: number) =>
    ["usage", "logs", filters, page, pageSize] as const,
  detail: (requestId: string) => ["usage", "detail", requestId] as const,
  pricing: ["usage", "pricing"] as const,
  devices: ["usage", "devices"] as const,
  sync: ["usage", "sync"] as const,
};

export const radarKeys = {
  all: ["codex-radar"] as const,
  snapshot: ["codex-radar", "snapshot"] as const,
};

function normalizeRefetchInterval(refetchMs: number): number | false {
  return refetchMs > 0 ? refetchMs : false;
}

function autoRefreshOptions(refetchMs: number) {
  return {
    refetchInterval: normalizeRefetchInterval(refetchMs),
    refetchIntervalInBackground: true,
  };
}

export function useUsageSummary(params: UsageQueryParams, refetchMs = 30000) {
  return useQuery({
    queryKey: usageKeys.summary(params),
    queryFn: () => fetchUsageSummary(params),
    ...autoRefreshOptions(refetchMs),
  });
}

export function useDailyTrends(params: UsageQueryParams, refetchMs = 30000) {
  return useQuery({
    queryKey: usageKeys.trends(params),
    queryFn: () => fetchDailyTrends(params),
    ...autoRefreshOptions(refetchMs),
  });
}

export function useProviderStats(params: UsageQueryParams, refetchMs = 30000) {
  return useQuery({
    queryKey: usageKeys.providerStats(params),
    queryFn: () => fetchProviderStats(params),
    ...autoRefreshOptions(refetchMs),
  });
}

export function useModelStats(params: UsageQueryParams, refetchMs = 30000) {
  return useQuery({
    queryKey: usageKeys.modelStats(params),
    queryFn: () => fetchModelStats(params),
    ...autoRefreshOptions(refetchMs),
  });
}

export function useRequestLogs(filters: LogFilters, page: number, pageSize: number, refetchMs = 30000) {
  return useQuery({
    queryKey: usageKeys.logs(filters, page, pageSize),
    queryFn: () => fetchRequestLogs(filters, page, pageSize),
    ...autoRefreshOptions(refetchMs),
  });
}

export function useRequestDetail(requestId: string | null) {
  return useQuery({
    queryKey: usageKeys.detail(requestId ?? ""),
    queryFn: () => fetchRequestDetail(requestId!),
    enabled: !!requestId,
  });
}

export function useModelPricing() {
  return useQuery({
    queryKey: usageKeys.pricing,
    queryFn: fetchModelPricing,
    staleTime: 5 * 60 * 1000,
  });
}

export function useRefreshModelPricing() {
  return useMutation({
    mutationFn: refreshModelPricing,
  });
}

export function useDevices() {
  return useQuery({
    queryKey: usageKeys.devices,
    queryFn: fetchDevices,
    staleTime: 30 * 1000,
  });
}

export function useCodexRadar() {
  return useQuery({
    queryKey: radarKeys.snapshot,
    queryFn: fetchCodexRadar,
    staleTime: 10 * 60 * 1000,
    refetchInterval: 10 * 60 * 1000,
    refetchIntervalInBackground: false,
    retry: 1,
  });
}

export function useSyncSessionLogs() {
  return useQuery({
    queryKey: usageKeys.sync,
    queryFn: syncSessionLogs,
    staleTime: 30 * 1000,
  });
}
