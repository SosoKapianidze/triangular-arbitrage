use super::{PriceMap, OrderRequest, ExchangeError};
use anyhow::Result;
use hmac::{Hmac, Mac};
use reqwest::{Client, ClientBuilder};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use std::env;
use std::time::Duration;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

type HmacSha256 = Hmac<Sha256>;

pub struct BybitClient {
    client: Client,
    api_key: String,
    secret_key: String,
    base_url: String,
    last_request_time: std::sync::Arc<std::sync::Mutex<DateTime<Utc>>>,
    rate_limiter: std::sync::Arc<tokio::sync::Semaphore>,
}

impl BybitClient {
    pub fn new() -> Result<Self> {
        let api_key = env::var("BYBIT_API_KEY")
            .map_err(|_| ExchangeError::MissingCredentials("BYBIT_API_KEY not found".to_string()))?;
        let secret_key = env::var("BYBIT_SECRET_KEY")
            .map_err(|_| ExchangeError::MissingCredentials("BYBIT_SECRET_KEY not found".to_string()))?;
        
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
            base_url: "https://api.bybit.com".to_string(),
            last_request_time: std::sync::Arc::new(std::sync::Mutex::new(Utc::now())),
            rate_limiter: std::sync::Arc::new(tokio::sync::Semaphore::new(10)),
        })
    }
    
    pub async fn get_ticker_prices(&self) -> Result<PriceMap> {
        let _permit = self.rate_limiter.acquire().await
            .map_err(|e| ExchangeError::RateLimitError(format!("Rate limit acquisition failed: {}", e)))?;
        
        self.enforce_rate_limit().await;
        
        let url = format!("{}/v5/market/tickers?category=spot", self.base_url);
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
        
        let data: Value = response.json().await
            .map_err(|e| ExchangeError::ParseError(format!("Failed to parse response: {}", e)))?;
        
        let mut price_map = HashMap::new();
        let now = Utc::now();
        
        if let Some(result) = data.get("result") {
            if let Some(list) = result.get("list").and_then(|l| l.as_array()) {
                for ticker in list {
                    if let (Some(symbol), Some(price_str)) = (
                        ticker.get("symbol").and_then(|s| s.as_str()),
                        ticker.get("lastPrice").and_then(|p| p.as_str())
                    ) {
                        if let Ok(price) = price_str.parse::<Decimal>() {
                            if price > Decimal::ZERO {
                                price_map.insert(symbol.to_string(), price);
                            }
                        }
                    }
                }
            }
        }
        
        // Update last request time
        if let Ok(mut last_time) = self.last_request_time.lock() {
            *last_time = now;
        }
        
        Ok(price_map)
    }
    
    pub async fn get_account_info(&self) -> Result<Value> {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let recv_window = 5000;
        
        let params = format!("timestamp={}&recv_window={}", timestamp, recv_window);
        let signature = self.generate_signature(&params)?;
        
        let url = format!("{}/v5/account/wallet-balance?{}&signature={}", 
                         self.base_url, params, signature);
        
        let response = self.client
            .get(&url)
            .header("X-BAPI-API-KEY", self.api_key.as_ref().unwrap())
            .header("X-BAPI-TIMESTAMP", timestamp.to_string())
            .header("X-BAPI-RECV-WINDOW", recv_window.to_string())
            .header("X-BAPI-SIGN", signature)
            .send()
            .await?;
            
        Ok(response.json().await?)
    }
    
    pub async fn place_order(&self, order: &OrderRequest) -> Result<Value> {
        let _permit = self.rate_limiter.acquire().await
            .map_err(|e| ExchangeError::RateLimitError(format!("Rate limit acquisition failed: {}", e)))?;
        
        self.enforce_rate_limit().await;
        
        let timestamp = chrono::Utc::now().timestamp_millis();
        let recv_window = 5000;
        
        let mut body = serde_json::json!({
            "category": "spot",
            "symbol": order.symbol,
            "side": match order.side {
                super::OrderSide::Buy => "Buy",
                super::OrderSide::Sell => "Sell",
            },
            "orderType": match order.order_type {
                super::OrderType::Market => "Market",
                super::OrderType::Limit => "Limit",
            },
            "qty": order.quantity.to_string(),
        });
        
        if let Some(price) = &order.price {
            body["price"] = serde_json::Value::String(price.to_string());
        }
        
        let body_str = serde_json::to_string(&body)
            .map_err(|e| ExchangeError::ParseError(format!("Failed to serialize order: {}", e)))?;
        
        // Correct Bybit signature format: timestamp + api_key + recv_window + body
        let sign_payload = format!("{}{}{}{}", timestamp, &self.api_key, recv_window, body_str);
        let signature = self.generate_signature(&sign_payload)?;
        
        let url = format!("{}/v5/order/create", self.base_url);
        
        let response = self.client
            .post(&url)
            .header("X-BAPI-API-KEY", &self.api_key)
            .header("X-BAPI-TIMESTAMP", timestamp.to_string())
            .header("X-BAPI-RECV-WINDOW", recv_window.to_string())
            .header("X-BAPI-SIGN", signature)
            .header("Content-Type", "application/json")
            .timeout(Duration::from_secs(15))
            .body(body_str)
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
    
    fn generate_signature(&self, payload: &str) -> Result<String> {
        let mut mac = HmacSha256::new_from_slice(self.secret_key.as_bytes())
            .map_err(|e| ExchangeError::SignatureError(format!("Invalid secret key: {}", e)))?;
        mac.update(payload.as_bytes());
        let result = mac.finalize();
        Ok(hex::encode(result.into_bytes()))
    }
    
    async fn enforce_rate_limit(&self) {
        // Bybit allows 120 requests per minute, so ~500ms between requests
        let min_interval = Duration::from_millis(500);
        
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