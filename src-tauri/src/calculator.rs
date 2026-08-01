//! Token 使用量与成本计算
//!
//! 移植自 cc-switch 的 proxy/usage/parser.rs + calculator.rs

use crate::schema::is_cache_inclusive_app;
use rust_decimal::Decimal;
use std::str::FromStr;

/// Token 使用量
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    pub model: Option<String>,
    pub message_id: Option<String>,
}

impl TokenUsage {
    pub fn has_billable_tokens(&self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.cache_read_tokens > 0
            || self.cache_creation_tokens > 0
    }
}

/// 成本明细
#[derive(Debug, Clone)]
pub struct CostBreakdown {
    pub input_cost: Decimal,
    pub output_cost: Decimal,
    pub cache_read_cost: Decimal,
    pub cache_creation_cost: Decimal,
    pub total_cost: Decimal,
}

/// 模型定价
#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub input_cost_per_million: Decimal,
    pub output_cost_per_million: Decimal,
    pub cache_read_cost_per_million: Decimal,
    pub cache_creation_cost_per_million: Decimal,
}

impl ModelPricing {
    pub fn from_strings(
        input: &str,
        output: &str,
        cache_read: &str,
        cache_creation: &str,
    ) -> std::result::Result<Self, rust_decimal::Error> {
        Ok(Self {
            input_cost_per_million: Decimal::from_str(input)?,
            output_cost_per_million: Decimal::from_str(output)?,
            cache_read_cost_per_million: Decimal::from_str(cache_read)?,
            cache_creation_cost_per_million: Decimal::from_str(cache_creation)?,
        })
    }
}

/// 成本计算器
pub struct CostCalculator;

impl CostCalculator {
    /// Claude 语义计算（input_tokens 已是 fresh）
    pub fn calculate(
        usage: &TokenUsage,
        pricing: &ModelPricing,
        cost_multiplier: Decimal,
    ) -> CostBreakdown {
        Self::calculate_with_cache_semantics(usage, pricing, cost_multiplier, false)
    }

    /// 按 app_type 选择输入 token 语义后计算成本
    pub fn calculate_for_app(
        app_type: &str,
        usage: &TokenUsage,
        pricing: &ModelPricing,
        cost_multiplier: Decimal,
    ) -> CostBreakdown {
        let input_includes_cache_read = is_cache_inclusive_app(app_type);
        Self::calculate_with_cache_semantics(
            usage,
            pricing,
            cost_multiplier,
            input_includes_cache_read,
        )
    }

    fn calculate_with_cache_semantics(
        usage: &TokenUsage,
        pricing: &ModelPricing,
        cost_multiplier: Decimal,
        input_includes_cache_read: bool,
    ) -> CostBreakdown {
        let million = Decimal::from(1_000_000);

        // cache-inclusive 应用：input_tokens 包含 cache_read + cache_creation，需扣除后再按输入价计费
        let billable_input_tokens = if input_includes_cache_read {
            usage
                .input_tokens
                .saturating_sub(usage.cache_read_tokens)
                .saturating_sub(usage.cache_creation_tokens)
        } else {
            usage.input_tokens
        };

        let input_cost =
            Decimal::from(billable_input_tokens) * pricing.input_cost_per_million / million;
        let output_cost =
            Decimal::from(usage.output_tokens) * pricing.output_cost_per_million / million;
        let cache_read_cost =
            Decimal::from(usage.cache_read_tokens) * pricing.cache_read_cost_per_million / million;
        let cache_creation_cost = Decimal::from(usage.cache_creation_tokens)
            * pricing.cache_creation_cost_per_million
            / million;

        let base_total = input_cost + output_cost + cache_read_cost + cache_creation_cost;
        let total_cost = base_total * cost_multiplier;

        CostBreakdown {
            input_cost,
            output_cost,
            cache_read_cost,
            cache_creation_cost,
            total_cost,
        }
    }

    pub fn try_calculate_for_app(
        app_type: &str,
        usage: &TokenUsage,
        pricing: Option<&ModelPricing>,
        cost_multiplier: Decimal,
    ) -> Option<CostBreakdown> {
        pricing.map(|p| Self::calculate_for_app(app_type, usage, p, cost_multiplier))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_semantics() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_creation_tokens: 100,
            ..Default::default()
        };
        let pricing = ModelPricing::from_strings("3.0", "15.0", "0.3", "3.75").unwrap();
        let cost = CostCalculator::calculate(&usage, &pricing, Decimal::from(1));
        assert_eq!(cost.input_cost, Decimal::from_str("0.003").unwrap());
        assert_eq!(cost.total_cost, Decimal::from_str("0.010935").unwrap());
    }

    #[test]
    fn test_cache_inclusive_app() {
        let usage = TokenUsage {
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_creation_tokens: 100,
            ..Default::default()
        };
        let pricing = ModelPricing::from_strings("3.0", "15.0", "0.3", "3.75").unwrap();
        let cost = CostCalculator::calculate_for_app("codex", &usage, &pricing, Decimal::from(1));
        assert_eq!(cost.input_cost, Decimal::from_str("0.0021").unwrap());
        assert_eq!(cost.total_cost, Decimal::from_str("0.010035").unwrap());
    }

    #[test]
    fn test_multiplier() {
        let usage = TokenUsage {
            input_tokens: 1000,
            ..Default::default()
        };
        let pricing = ModelPricing::from_strings("3.0", "0", "0", "0").unwrap();
        let cost = CostCalculator::calculate(&usage, &pricing, Decimal::from_str("1.5").unwrap());
        assert_eq!(cost.total_cost, Decimal::from_str("0.0045").unwrap());
    }
}
