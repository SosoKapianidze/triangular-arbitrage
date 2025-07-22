use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub trading: TradingConfig,
    pub risk: RiskConfig,
    pub exchanges: ExchangeConfig,
    pub monitoring: MonitoringConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    pub min_profit_threshold: Decimal,
    pub max_position_size: Decimal,
    pub trading_pairs: Vec<String>,
    pub enable_execution: bool,
    pub max_slippage_percentage: Decimal,
    pub min_liquidity_usd: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_daily_loss: Decimal,
    pub max_open_positions: u32,
    pub stop_loss_percentage: Decimal,
    pub max_consecutive_errors: u32,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_reset_minutes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeConfig {
    pub binance_enabled: bool,
    pub bybit_enabled: bool,
    pub rate_limit_ms: u64,
    pub request_timeout_seconds: u64,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    pub log_level: String,
    pub enable_metrics: bool,
    pub alert_on_errors: bool,
    pub price_staleness_seconds: i64,
    pub opportunity_history_days: i64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trading: TradingConfig {
                min_profit_threshold: Decimal::from_str_exact("0.5").unwrap(),
                max_position_size: Decimal::from_str_exact("1000.0").unwrap(),
                trading_pairs: vec![
                    "BTCUSDT".to_string(),
                    "ETHUSDT".to_string(),
                    "BNBUSDT".to_string(),
                ],
                enable_execution: false, // Disabled by default for safety
                max_slippage_percentage: Decimal::from_str_exact("0.1").unwrap(), // 0.1%
                min_liquidity_usd: Decimal::from_str_exact("10000.0").unwrap(), // $10k minimum liquidity
            },
            risk: RiskConfig {
                max_daily_loss: Decimal::from_str_exact("100.0").unwrap(),
                max_open_positions: 3,
                stop_loss_percentage: Decimal::from_str_exact("2.0").unwrap(),
                max_consecutive_errors: 10,
                circuit_breaker_threshold: 5,
                circuit_breaker_reset_minutes: 5,
            },
            exchanges: ExchangeConfig {
                binance_enabled: true,
                bybit_enabled: true,
                rate_limit_ms: 250, // Conservative rate limiting
                request_timeout_seconds: 10,
                max_retries: 3,
            },
            monitoring: MonitoringConfig {
                log_level: "info".to_string(),
                enable_metrics: true,
                alert_on_errors: true,
                price_staleness_seconds: 30,
                opportunity_history_days: 7,
            },
        }
    }
}

impl Config {
    pub fn load_from_file(path: &str) -> Result<Self> {
        if !std::path::Path::new(path).exists() {
            let default_config = Self::default();
            default_config.save_to_file(path)?;
            log::info!("Created default config file at {}", path);
            return Ok(default_config);
        }
        
        let content = fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&content)?;
        
        // Validate config
        config.validate()?;
        
        Ok(config)
    }
    
    pub fn save_to_file(&self, path: &str) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
    
    pub fn validate(&self) -> Result<()> {
        // Validate trading config
        if self.trading.min_profit_threshold < Decimal::ZERO {
            return Err(anyhow::anyhow!("min_profit_threshold cannot be negative"));
        }
        
        if self.trading.max_position_size <= Decimal::ZERO {
            return Err(anyhow::anyhow!("max_position_size must be positive"));
        }
        
        if self.trading.trading_pairs.is_empty() {
            return Err(anyhow::anyhow!("trading_pairs cannot be empty"));
        }
        
        if self.trading.max_slippage_percentage < Decimal::ZERO || 
           self.trading.max_slippage_percentage > Decimal::from(10) {
            return Err(anyhow::anyhow!("max_slippage_percentage must be between 0 and 10"));
        }
        
        // Validate risk config
        if self.risk.max_consecutive_errors == 0 {
            return Err(anyhow::anyhow!("max_consecutive_errors must be greater than 0"));
        }
        
        if self.risk.circuit_breaker_threshold == 0 {
            return Err(anyhow::anyhow!("circuit_breaker_threshold must be greater than 0"));
        }
        
        // Validate exchange config
        if !self.exchanges.binance_enabled && !self.exchanges.bybit_enabled {
            return Err(anyhow::anyhow!("At least one exchange must be enabled"));
        }
        
        if self.exchanges.request_timeout_seconds == 0 {
            return Err(anyhow::anyhow!("request_timeout_seconds must be greater than 0"));
        }
        
        Ok(())
    }
    
    pub fn get_trading_fee(&self, exchange: &str) -> Decimal {
        match exchange.to_lowercase().as_str() {
            "binance" => Decimal::from_str_exact("0.001").unwrap(), // 0.1%
            "bybit" => Decimal::from_str_exact("0.001").unwrap(),   // 0.1%
            _ => Decimal::from_str_exact("0.002").unwrap(),         // 0.2% default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    
    #[test]
    fn test_default_config_validation() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }
    
    #[test]
    fn test_config_save_and_load() {
        let temp_file = NamedTempFile::new().unwrap();
        let temp_path = temp_file.path().to_str().unwrap();
        
        let original_config = Config::default();
        original_config.save_to_file(temp_path).unwrap();
        
        let loaded_config = Config::load_from_file(temp_path).unwrap();
        
        assert_eq!(original_config.trading.min_profit_threshold, loaded_config.trading.min_profit_threshold);
        assert_eq!(original_config.risk.max_daily_loss, loaded_config.risk.max_daily_loss);
    }
    
    #[test]
    fn test_invalid_config_validation() {
        let mut config = Config::default();
        
        // Test negative profit threshold
        config.trading.min_profit_threshold = Decimal::from(-1);
        assert!(config.validate().is_err());
        
        // Test empty trading pairs
        config = Config::default();
        config.trading.trading_pairs.clear();
        assert!(config.validate().is_err());
        
        // Test both exchanges disabled
        config = Config::default();
        config.exchanges.binance_enabled = false;
        config.exchanges.bybit_enabled = false;
        assert!(config.validate().is_err());
    }
}