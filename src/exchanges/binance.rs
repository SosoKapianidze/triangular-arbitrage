use super::{PriceMap, TickerPrice, OrderRequest, ExchangeError};
use anyhow::Result;
use hmac::{Hmac, Mac};
use reqwest::{Client, ClientBuilder};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use std::env;
use std::time::Duration;
use url::Url;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

type HmacSha256 = Hmac<Sha256>;

pub struct BinanceClient {
    client: Client,
    api_key: String,
    secret_key: String,
    base_url: String,
    last_request_time: std::sync::Arc<std::sync::Mutex<DateTime<Utc>>>,
    rate_limiter: std::sync::Arc<tokio::sync::Semaphore>,
}

impl BinanceClient {
    pub fn new() -> Result<Self> {
        let api_key = env::var("BINANCE_API_KEY")
            .map_err(|_| ExchangeError::MissingCredentials("BINANCE_API_KEY not found".to_string()))?;
        let secret_key = env::var("BINANCE_SECRET_KEY")
            .map_err(|_| ExchangeError::MissingCredentials("BINANCE_SECRET_KEY not found".to_string()))?;
        
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10)
            .build()
            .map_err(|e| ExchangeError::NetworkError(format!("Failed to create client: {}", e)))?;
        
        Ok(Self {
            client,
            api_key,
            secret_key,
            base_url: "https://api.binance.com".to_string(),
            last_request_time: std::sync::Arc::new(std::sync::Mutex::new(Utc::now())),
            rate_limiter: std::sync::Arc::new(tokio::sync::Semaphore::new(10)), // 10 requests per batch
        })
    }
    
    pub async fn get_ticker_prices(&self) -> Result<PriceMap> {
        let _permit = self.rate_limiter.acquire().await
            .map_err(|e| ExchangeError::RateLimitError(format!("Rate limit acquisition failed: {}", e)))?;
        
        self.enforce_rate_limit().await;
        
        let url = format!("{}/api/v3/ticker/price", self.base_url);
        let response = self.client.get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| ExchangeError::NetworkError(format!("Request failed: {}", e)))?;
        
        if !response.status().is_success() {
            return Err(ExchangeError::ApiError(format!(
                "HTTP {}: {}", 
                response.status(), 
                response.text().await.unwrap_or_default()
            )).into());
        }
        
        let tickers: Vec<TickerPrice> = response.json().await
            .map_err(|e| ExchangeError::ParseError(format!("Failed to parse response: {}", e)))?;
        
        let mut price_map = HashMap::new();
        let now = Utc::now();
        
        for ticker in tickers {
            if ticker.price > Decimal::ZERO {
                price_map.insert(ticker.symbol, ticker.price);
            }
        }
        
        // Update last request time
        if let Ok(mut last_time) = self.last_request_time.lock() {
            *last_time = now;
        }
        
        Ok(price_map)
    }
    
    pub async fn get_account_info(&self) -> Result<Value> {
        let _permit = self.rate_limiter.acquire().await
            .map_err(|e| ExchangeError::RateLimitError(format!("Rate limit acquisition failed: {}", e)))?;
        
        self.enforce_rate_limit().await;
        
        let endpoint = "/api/v3/account";
        let timestamp = chrono::Utc::now().timestamp_millis();
        let query_string = format!("timestamp={}", timestamp);
        
        let signature = self.generate_signature(&query_string)?;
        let url = format!("{}{}?{}&signature={}", self.base_url, endpoint, query_string, signature);
        
        let response = self.client
            .get(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ExchangeError::NetworkError(format!("Account info request failed: {}", e)))?;
        
        if !response.status().is_success() {
            return Err(ExchangeError::ApiError(format!(
                "HTTP {}: {}", 
                response.status(), 
                response.text().await.unwrap_or_default()
            )).into());
        }
            
        Ok(response.json().await
            .map_err(|e| ExchangeError::ParseError(format!("Failed to parse account info: {}", e)))?)
    }
    
    pub async fn place_order(&self, order: &OrderRequest) -> Result<Value> {
        let _permit = self.rate_limiter.acquire().await
            .map_err(|e| ExchangeError::RateLimitError(format!("Rate limit acquisition failed: {}", e)))?;
        
        self.enforce_rate_limit().await;
        
        let endpoint = "/api/v3/order";
        let timestamp = chrono::Utc::now().timestamp_millis();
        
        let mut params = vec![
            ("symbol", order.symbol.clone()),
            ("side", match order.side {
                super::OrderSide::Buy => "BUY".to_string(),
                super::OrderSide::Sell => "SELL".to_string(),
            }),
            ("type", match order.order_type {
                super::OrderType::Market => "MARKET".to_string(),
                super::OrderType::Limit => "LIMIT".to_string(),
            }),
            ("quantity", order.quantity.to_string()),
            ("timestamp", timestamp.to_string()),
        ];
        
        if let Some(price) = &order.price {
            params.push(("price", price.to_string()));
            params.push(("timeInForce", "GTC".to_string()));
        }
        
        let query_string = params.iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
            
        let signature = self.generate_signature(&query_string)?;
        let url = format!("{}{}?{}&signature={}", self.base_url, endpoint, query_string, signature);
        
        let response = self.client
            .post(&url)
            .header("X-MBX-APIKEY", &self.api_key)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ExchangeError::NetworkError(format!("Order placement failed: {}", e)))?;
        
        if !response.status().is_success() {
            return Err(ExchangeError::ApiError(format!(
                "Order failed - HTTP {}: {}", 
                response.status(), 
                response.text().await.unwrap_or_default()
            )).into());
        }
            
        Ok(response.json().await
            .map_err(|e| ExchangeError::ParseError(format!("Failed to parse order response: {}", e)))?)
    }
    
    fn generate_signature(&self, query_string: &str) -> Result<String> {
        let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
            .map_err(|e| ExchangeError::SignatureError(format!("Invalid secret key: {}", e)))?;
        mac.update(query_string.as_bytes());
        let result = mac.finalize();
        Ok(hex::encode(result.into_bytes()))
    }
    
    async fn enforce_rate_limit(&self) {
        // Binance allows 1200 requests per minute, so ~50ms between requests
        let min_interval = Duration::from_millis(50);
        
        if let Ok(last_time) = self.last_request_time.lock() {
            let elapsed = Utc::now().signed_duration_since(*last_time);
            if let Ok(elapsed_std) = elapsed.to_std() {
                if elapsed_std < min_interval {
                    let sleep_time = min_interval - elapsed_std;
                    tokio::time::sleep(sleep_time).await;
                }
            }
        }
    }
}