pub mod exchanges;
pub mod arbitrage;
pub mod config;

use crate::arbitrage::ArbitrageEngine;
use crate::exchanges::{binance::BinanceClient, bybit::BybitClient, ExchangeError};
use anyhow::Result;
use log::{info, error, warn};
use backoff::{ExponentialBackoff, future::retry};
use std::time::Duration;
use tokio::time::timeout;

pub struct ArbitrageBot {
    binance: BinanceClient,
    bybit: BybitClient,
    engine: ArbitrageEngine,
}

impl ArbitrageBot {
    pub async fn new() -> Result<Self> {
        let binance = BinanceClient::new()
            .map_err(|e| anyhow::anyhow!("Failed to create Binance client: {}", e))?;
        let bybit = BybitClient::new()
            .map_err(|e| anyhow::anyhow!("Failed to create Bybit client: {}", e))?;
        let engine = ArbitrageEngine::new();
        
        // Test connectivity
        info!("Testing exchange connectivity...");
        
        let binance_test = binance.get_ticker_prices();
        let bybit_test = bybit.get_ticker_prices();
        
        match tokio::try_join!(binance_test, bybit_test) {
            Ok((binance_prices, bybit_prices)) => {
                info!("Connectivity test successful. Binance: {} pairs, Bybit: {} pairs", 
                      binance_prices.len(), bybit_prices.len());
            }
            Err(e) => {
                error!("Connectivity test failed: {}", e);
                return Err(anyhow::anyhow!("Exchange connectivity test failed: {}", e));
            }
        }
        
        Ok(Self {
            binance,
            bybit,
            engine,
        })
    }
    
    pub async fn run(&self) -> Result<()> {
        info!("Starting triangular arbitrage bot...");
        
        let mut consecutive_errors = 0;
        let max_consecutive_errors = 10;
        
        loop {
            match timeout(
                Duration::from_secs(30), // 30 second timeout for each scan
                self.scan_opportunities_with_retry()
            ).await {
                Ok(Ok(())) => {
                    consecutive_errors = 0;
                    tokio::time::sleep(Duration::from_millis(250)).await; // Reduced frequency for safety
                }
                Ok(Err(e)) => {
                    consecutive_errors += 1;
                    error!("Error scanning opportunities (attempt {}): {}", consecutive_errors, e);
                    
                    if consecutive_errors >= max_consecutive_errors {
                        error!("Too many consecutive errors ({}), stopping bot", consecutive_errors);
                        return Err(anyhow::anyhow!("Bot stopped due to excessive errors"));
                    }
                    
                    // Exponential backoff on errors
                    let sleep_duration = Duration::from_secs(2_u64.pow(consecutive_errors.min(6)));
                    warn!("Sleeping for {:?} before retry", sleep_duration);
                    tokio::time::sleep(sleep_duration).await;
                }
                Err(_) => {
                    error!("Scan timed out after 30 seconds");
                    consecutive_errors += 1;
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
    
    async fn scan_opportunities_with_retry(&self) -> Result<()> {
        let backoff = ExponentialBackoff {
            max_elapsed_time: Some(Duration::from_secs(60)),
            max_interval: Duration::from_secs(10),
            ..Default::default()
        };
        
        retry(backoff, || async {
            self.scan_opportunities().await.map_err(|e| {
                match e.downcast_ref::<ExchangeError>() {
                    Some(ExchangeError::NetworkError(_)) => backoff::Error::transient(e),
                    Some(ExchangeError::RateLimitError(_)) => backoff::Error::transient(e),
                    _ => backoff::Error::permanent(e),
                }
            })
        }).await
    }
    
    async fn scan_opportunities(&self) -> Result<()> {
        // Parallel API calls for better performance
        let (binance_result, bybit_result) = tokio::join!(
            timeout(Duration::from_secs(10), self.binance.get_ticker_prices()),
            timeout(Duration::from_secs(10), self.bybit.get_ticker_prices())
        );
        
        let binance_prices = binance_result
            .map_err(|_| anyhow::anyhow!("Binance API timeout"))?
            .map_err(|e| anyhow::anyhow!("Binance API error: {}", e))?;
            
        let bybit_prices = bybit_result
            .map_err(|_| anyhow::anyhow!("Bybit API timeout"))?
            .map_err(|e| anyhow::anyhow!("Bybit API error: {}", e))?;
        
        if binance_prices.is_empty() || bybit_prices.is_empty() {
            return Err(anyhow::anyhow!("Received empty price data from exchanges"));
        }
        
        info!("Received prices: Binance={}, Bybit={}", binance_prices.len(), bybit_prices.len());
        
        self.engine.analyze_opportunities(&binance_prices, &bybit_prices).await?;
        
        Ok(())
    }
}