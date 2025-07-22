use crate::exchanges::{PriceMap, OrderRequest, OrderSide, OrderType, TradingFees};
use anyhow::Result;
use log::{info, warn};
use rust_decimal::Decimal;
use rust_decimal::prelude::*;
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ArbitrageOpportunity {
    pub exchange: String,
    pub path: Vec<String>,
    pub profit_percentage: Decimal,
    pub net_profit_percentage: Decimal, // After fees
    pub required_amount: Decimal,
    pub estimated_profit_usd: Decimal,
    pub risk_score: f32,
    pub execution_steps: Vec<ExecutionStep>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ExecutionStep {
    pub action: String,
    pub symbol: String,
    pub side: OrderSide,
    pub quantity: Decimal,
    pub expected_price: Decimal,
    pub fees: Decimal,
}

pub struct ArbitrageEngine {
    min_profit_threshold: Decimal,
    max_position_size: Decimal,
    trading_pairs: Vec<String>,
    fees: TradingFees,
    price_cache: Arc<DashMap<String, (Decimal, DateTime<Utc>)>>,
    opportunity_history: Arc<DashMap<String, Vec<ArbitrageOpportunity>>>,
    circuit_breaker: CircuitBreaker,
}

#[derive(Debug, Clone)]
struct CircuitBreaker {
    failure_count: Arc<std::sync::Mutex<u32>>,
    last_failure: Arc<std::sync::Mutex<Option<DateTime<Utc>>>>,
    threshold: u32,
    reset_timeout: chrono::Duration,
}

impl CircuitBreaker {
    fn new(threshold: u32, reset_timeout_minutes: i64) -> Self {
        Self {
            failure_count: Arc::new(std::sync::Mutex::new(0)),
            last_failure: Arc::new(std::sync::Mutex::new(None)),
            threshold,
            reset_timeout: chrono::Duration::minutes(reset_timeout_minutes),
        }
    }
    
    fn is_open(&self) -> bool {
        let count = *self.failure_count.lock().unwrap();
        if count >= self.threshold {
            if let Some(last_failure) = *self.last_failure.lock().unwrap() {
                let elapsed = Utc::now().signed_duration_since(last_failure);
                return elapsed < self.reset_timeout;
            }
        }
        false
    }
    
    fn record_failure(&self) {
        *self.failure_count.lock().unwrap() += 1;
        *self.last_failure.lock().unwrap() = Some(Utc::now());
    }
    
    fn reset(&self) {
        *self.failure_count.lock().unwrap() = 0;
        *self.last_failure.lock().unwrap() = None;
    }
}

impl ArbitrageEngine {
    pub fn new() -> Self {
        Self {
            min_profit_threshold: Decimal::from_str_exact("0.5").unwrap(), // 0.5% minimum profit
            max_position_size: Decimal::from_str_exact("1000.0").unwrap(), // $1000 max position
            trading_pairs: vec![
                "BTCUSDT".to_string(),
                "ETHUSDT".to_string(),
                "BNBUSDT".to_string(),
                "ADAUSDT".to_string(),
                "DOTUSDT".to_string(),
                "SOLUSDT".to_string(),
            ],
            fees: TradingFees::default(),
            price_cache: Arc::new(DashMap::new()),
            opportunity_history: Arc::new(DashMap::new()),
            circuit_breaker: CircuitBreaker::new(5, 5), // 5 failures, 5 minute reset
        }
    }
    
    pub async fn analyze_opportunities(
        &self,
        binance_prices: &PriceMap,
        bybit_prices: &PriceMap,
    ) -> Result<()> {
        if self.circuit_breaker.is_open() {
            warn!("Circuit breaker is open, skipping opportunity analysis");
            return Ok(());
        }
        
        // Check for cross-exchange arbitrage opportunities
        for pair in &self.trading_pairs {
            if let (Some(binance_price), Some(bybit_price)) = 
                (binance_prices.get(pair), bybit_prices.get(pair)) {
                
                // Validate price freshness
                if !self.is_price_fresh(pair, *binance_price, *bybit_price) {
                    continue;
                }
                
                let price_diff = (binance_price - bybit_price).abs();
                let avg_price = (binance_price + bybit_price) / Decimal::TWO;
                
                // Prevent division by zero
                if avg_price == Decimal::ZERO {
                    warn!("Zero average price for pair: {}", pair);
                    continue;
                }
                
                let gross_profit_percentage = (price_diff / avg_price) * Decimal::ONE_HUNDRED;
                
                // Calculate net profit after fees
                let total_fees = self.fees.taker_fee * Decimal::TWO; // Two trades
                let net_profit_percentage = gross_profit_percentage - (total_fees * Decimal::ONE_HUNDRED);
                
                if net_profit_percentage > self.min_profit_threshold {
                    let (sell_exchange, buy_exchange, sell_price, buy_price) = if binance_price > bybit_price {
                        ("Binance", "Bybit", *binance_price, *bybit_price)
                    } else {
                        ("Bybit", "Binance", *bybit_price, *binance_price)
                    };
                    
                    let quantity = self.max_position_size / sell_price;
                    let estimated_profit = (sell_price - buy_price) * quantity - 
                                         (sell_price * quantity * self.fees.taker_fee) -
                                         (buy_price * quantity * self.fees.taker_fee);
                    
                    let execution_steps = vec![
                        ExecutionStep {
                            action: format!("Sell on {}", sell_exchange),
                            symbol: pair.clone(),
                            side: OrderSide::Sell,
                            quantity,
                            expected_price: sell_price,
                            fees: sell_price * quantity * self.fees.taker_fee,
                        },
                        ExecutionStep {
                            action: format!("Buy on {}", buy_exchange),
                            symbol: pair.clone(),
                            side: OrderSide::Buy,
                            quantity,
                            expected_price: buy_price,
                            fees: buy_price * quantity * self.fees.taker_fee,
                        },
                    ];
                    
                    let opportunity = ArbitrageOpportunity {
                        exchange: format!("{}->{}", sell_exchange, buy_exchange),
                        path: vec![
                            format!("Sell {} on {} at {}", pair, sell_exchange, sell_price),
                            format!("Buy {} on {} at {}", pair, buy_exchange, buy_price)
                        ],
                        profit_percentage: gross_profit_percentage,
                        net_profit_percentage,
                        required_amount: self.max_position_size,
                        estimated_profit_usd: estimated_profit,
                        risk_score: self.calculate_risk_score(&price_diff, &avg_price),
                        execution_steps,
                        timestamp: Utc::now(),
                    };
                    
                    info!("Arbitrage opportunity found: {:?}", opportunity);
                    // self.execute_arbitrage(&opportunity).await?;
                }
            }
        }
        
        // Check for triangular arbitrage within each exchange
        self.check_triangular_arbitrage(binance_prices, "Binance").await?;
        self.check_triangular_arbitrage(bybit_prices, "Bybit").await?;
        
        Ok(())
    }
    
    async fn check_triangular_arbitrage(&self, prices: &PriceMap, exchange: &str) -> Result<()> {
        // Common triangular arbitrage paths
        let triangular_paths = vec![
            ("BTCUSDT", "ETHBTC", "ETHUSDT"),
            ("BTCUSDT", "BNBBTC", "BNBUSDT"),
            ("ETHUSDT", "ADAETH", "ADAUSDT"),
        ];
        
        for (pair1, pair2, pair3) in triangular_paths {
            if let (Some(price1), Some(price2), Some(price3)) = 
                (prices.get(pair1), prices.get(pair2), prices.get(pair3)) {
                
                // Prevent division by zero
                if *price1 == Decimal::ZERO || *price2 == Decimal::ZERO || *price3 == Decimal::ZERO {
                    continue;
                }
                
                // Calculate triangular arbitrage profit
                // Example: BTCUSDT=50000, ETHBTC=0.06, ETHUSDT=3000
                // Forward path: USDT -> BTC -> ETH -> USDT
                // 1 USDT -> 1/50000 BTC -> (1/50000)*0.06 ETH -> (1/50000)*0.06*3000 USDT = 0.0036 USDT
                let forward_result = (Decimal::ONE / price1) * price2 * price3;
                let forward_gross_profit = (forward_result - Decimal::ONE) * Decimal::ONE_HUNDRED;
                
                // Account for three trading fees (3 trades in triangular arbitrage)
                let triangular_fees = self.fees.taker_fee * Decimal::from(3);
                let forward_net_profit = forward_gross_profit - (triangular_fees * Decimal::ONE_HUNDRED);
                
                // Reverse path: USDT -> ETH -> BTC -> USDT  
                // 1 USDT -> 1/3000 ETH -> (1/3000)/0.06 BTC -> ((1/3000)/0.06)*50000 USDT
                let reverse_result = (Decimal::ONE / price3) * (Decimal::ONE / price2) * price1;
                let reverse_gross_profit = (reverse_result - Decimal::ONE) * Decimal::ONE_HUNDRED;
                let reverse_net_profit = reverse_gross_profit - (triangular_fees * Decimal::ONE_HUNDRED);
                
                if forward_net_profit > self.min_profit_threshold {
                    let base_currency = pair1.replace("USDT", "");
                    let quote_currency = pair3.replace("USDT", "");
                    
                    let usdt_amount = self.max_position_size;
                    let estimated_profit = usdt_amount * (forward_result - Decimal::ONE) - 
                                         (usdt_amount * triangular_fees);
                    
                    let execution_steps = vec![
                        ExecutionStep {
                            action: format!("Buy {} with USDT", base_currency),
                            symbol: pair1.to_string(),
                            side: OrderSide::Buy,
                            quantity: usdt_amount / price1,
                            expected_price: *price1,
                            fees: usdt_amount * self.fees.taker_fee,
                        },
                        ExecutionStep {
                            action: format!("Trade {} to {}", base_currency, quote_currency),
                            symbol: pair2.to_string(),
                            side: OrderSide::Sell,
                            quantity: usdt_amount / price1,
                            expected_price: *price2,
                            fees: (usdt_amount / price1) * price2 * self.fees.taker_fee,
                        },
                        ExecutionStep {
                            action: format!("Sell {} for USDT", quote_currency),
                            symbol: pair3.to_string(),
                            side: OrderSide::Sell,
                            quantity: (usdt_amount / price1) * price2,
                            expected_price: *price3,
                            fees: ((usdt_amount / price1) * price2) * price3 * self.fees.taker_fee,
                        },
                    ];
                    
                    let opportunity = ArbitrageOpportunity {
                        exchange: exchange.to_string(),
                        path: vec![
                            format!("Buy {} with USDT at {}", base_currency, price1),
                            format!("Trade {} to {} via {} at {}", base_currency, quote_currency, pair2, price2),
                            format!("Sell {} for USDT at {}", quote_currency, price3),
                        ],
                        profit_percentage: forward_gross_profit,
                        net_profit_percentage: forward_net_profit,
                        required_amount: self.max_position_size,
                        estimated_profit_usd: estimated_profit,
                        risk_score: self.calculate_triangular_risk_score(price1, price2, price3),
                        execution_steps,
                        timestamp: Utc::now(),
                    };
                    
                    info!("Triangular arbitrage opportunity (forward): {:?}", opportunity);
                    self.record_opportunity(&opportunity);
                } else if reverse_net_profit > self.min_profit_threshold {
                    let base_currency = pair1.replace("USDT", "");
                    let quote_currency = pair3.replace("USDT", "");
                    
                    let usdt_amount = self.max_position_size;
                    let estimated_profit = usdt_amount * (reverse_result - Decimal::ONE) - 
                                         (usdt_amount * triangular_fees);
                    
                    let execution_steps = vec![
                        ExecutionStep {
                            action: format!("Buy {} with USDT", quote_currency),
                            symbol: pair3.to_string(),
                            side: OrderSide::Buy,
                            quantity: usdt_amount / price3,
                            expected_price: *price3,
                            fees: usdt_amount * self.fees.taker_fee,
                        },
                        ExecutionStep {
                            action: format!("Trade {} to {}", quote_currency, base_currency),
                            symbol: pair2.to_string(),
                            side: OrderSide::Buy,
                            quantity: (usdt_amount / price3) / price2,
                            expected_price: *price2,
                            fees: (usdt_amount / price3) * self.fees.taker_fee,
                        },
                        ExecutionStep {
                            action: format!("Sell {} for USDT", base_currency),
                            symbol: pair1.to_string(),
                            side: OrderSide::Sell,
                            quantity: (usdt_amount / price3) / price2,
                            expected_price: *price1,
                            fees: ((usdt_amount / price3) / price2) * price1 * self.fees.taker_fee,
                        },
                    ];
                    
                    let opportunity = ArbitrageOpportunity {
                        exchange: exchange.to_string(),
                        path: vec![
                            format!("Buy {} with USDT at {}", quote_currency, price3),
                            format!("Trade {} to {} via {} at {}", quote_currency, base_currency, pair2, price2),
                            format!("Sell {} for USDT at {}", base_currency, price1),
                        ],
                        profit_percentage: reverse_gross_profit,
                        net_profit_percentage: reverse_net_profit,
                        required_amount: self.max_position_size,
                        estimated_profit_usd: estimated_profit,
                        risk_score: self.calculate_triangular_risk_score(price1, price2, price3),
                        execution_steps,
                        timestamp: Utc::now(),
                    };
                    
                    info!("Triangular arbitrage opportunity (reverse): {:?}", opportunity);
                    self.record_opportunity(&opportunity);
                }
            }
        }
        
        Ok(())
    }
    
    fn is_price_fresh(&self, symbol: &str, price1: Decimal, price2: Decimal) -> bool {
        // Check if prices have been updated recently and are reasonable
        let price_age_limit = chrono::Duration::seconds(30);
        let now = Utc::now();
        
        if let Some((cached_price, timestamp)) = self.price_cache.get(symbol) {
            let age = now.signed_duration_since(*timestamp);
            if age > price_age_limit {
                return false;
            }
        }
        
        // Update cache
        self.price_cache.insert(symbol.to_string(), ((price1 + price2) / Decimal::TWO, now));
        
        // Check for reasonable price variance (not more than 10% difference)
        let max_variance = Decimal::from_str_exact("0.1").unwrap();
        let price_diff = (price1 - price2).abs();
        let avg_price = (price1 + price2) / Decimal::TWO;
        
        if avg_price > Decimal::ZERO {
            let variance = price_diff / avg_price;
            return variance <= max_variance;
        }
        
        false
    }
    
    fn calculate_risk_score(&self, price_diff: &Decimal, avg_price: &Decimal) -> f32 {
        // Higher price difference = higher risk due to potential stale data or market volatility
        if *avg_price == Decimal::ZERO {
            return 1.0; // Maximum risk
        }
        
        let variance = price_diff / avg_price;
        let variance_f32 = variance.to_f32().unwrap_or(1.0);
        
        // Risk score from 0.0 (low risk) to 1.0 (high risk)
        (variance_f32 * 10.0).min(1.0)
    }
    
    fn calculate_triangular_risk_score(&self, price1: &Decimal, price2: &Decimal, price3: &Decimal) -> f32 {
        // Triangular arbitrage has higher complexity risk
        let base_risk = 0.3; // Base risk for triangular trades
        
        // Add risk based on price volatility estimation
        let prices = vec![*price1, *price2, *price3];
        let avg = prices.iter().sum::<Decimal>() / Decimal::from(prices.len());
        
        if avg == Decimal::ZERO {
            return 1.0;
        }
        
        let variance = prices.iter()
            .map(|p| (*p - avg).abs() / avg)
            .map(|v| v.to_f32().unwrap_or(0.0))
            .sum::<f32>() / prices.len() as f32;
        
        (base_risk + variance).min(1.0)
    }
    
    fn record_opportunity(&self, opportunity: &ArbitrageOpportunity) {
        let key = format!("{}_{}", opportunity.exchange, opportunity.timestamp.format("%Y%m%d"));
        
        self.opportunity_history
            .entry(key)
            .or_insert_with(Vec::new)
            .push(opportunity.clone());
        
        // Cleanup old records (keep only last 7 days)
        let cutoff = Utc::now() - chrono::Duration::days(7);
        self.opportunity_history.retain(|_, opportunities| {
            opportunities.retain(|opp| opp.timestamp > cutoff);
            !opportunities.is_empty()
        });
    }
    
    pub async fn execute_arbitrage(&self, opportunity: &ArbitrageOpportunity) -> Result<()> {
        if self.circuit_breaker.is_open() {
            warn!("Circuit breaker is open, skipping arbitrage execution");
            return Ok(());
        }
        
        if opportunity.risk_score > 0.7 {
            warn!("Risk score too high ({:.2}), skipping execution", opportunity.risk_score);
            return Ok(());
        }
        
        warn!("Arbitrage execution is disabled for safety. Opportunity: {:?}", opportunity);
        // Implementation would go here for actual trading
        // This requires careful risk management and testing
        Ok(())
    }
}