/// 格式化工具函数

export type FormatLocale = "zh-CN" | "en-US";

export function getFormatLocale(): FormatLocale {
  if (typeof document !== "undefined" && document.documentElement.lang === "zh-CN") {
    return "zh-CN";
  }
  return "en-US";
}

export function fmtInt(n: number, locale: FormatLocale = getFormatLocale()): string {
  return new Intl.NumberFormat(locale).format(n);
}

export function fmtUsd(s: string | number): string {
  const n = typeof s === "string" ? parseFloat(s) : s;
  if (isNaN(n)) return "$0.00";
  if (n < 0.01) return `$${n.toFixed(6)}`;
  if (n < 1) return `$${n.toFixed(4)}`;
  return `$${n.toFixed(2)}`;
}

export function fmtTokens(n: number, locale: FormatLocale = getFormatLocale()): string {
  if (locale === "zh-CN" && n >= 100_000_000) return `${(n / 100_000_000).toFixed(2)}亿`;
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(2)}K`;
  return new Intl.NumberFormat(locale).format(n);
}

export function fmtDateTime(unixSeconds: number, locale: FormatLocale = getFormatLocale()): string {
  return new Date(unixSeconds * 1000).toLocaleString(locale);
}

export function fmtPercent(n: number, digits = 1): string {
  return `${n.toFixed(digits)}%`;
}

export function fmtLatency(ms: number): string {
  if (ms < 1000) return `${ms.toFixed(0)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}
