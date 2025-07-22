pub mod binance;
pub mod bybit;
pub mod order_book;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickerPrice {
    pub symbol: String,
    pub price: Decimal,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub symbol: String,
    pub bids: Vec<(Decimal, Decimal)>, // (price, quantity)
    pub asks: Vec<(Decimal, Decimal)>, // (price, quantity)
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct TradingFees {
    pub maker_fee: Decimal,
    pub taker_fee: Decimal,
    pub withdrawal_fee: Decimal,
}

#[derive(Error, Debug)]
pub enum ExchangeError {
    #[error("Missing credentials: {0}")]
    MissingCredentials(String),
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("API error: {0}")]
    ApiError(String),
    
    #[error("Parse error: {0}")]
    ParseError(String),
    
    #[error("Rate limit error: {0}")]
    RateLimitError(String),
    
    #[error("Signature error: {0}")]
    SignatureError(String),
    
    #[error("Insufficient balance: {0}")]
    InsufficientBalance(String),
}

#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub symbol: String,
    pub side: OrderSide,
    pub quantity: Decimal,
    pub price: Option<Decimal>,
    pub order_type: OrderType,
}

#[derive(Debug, Clone)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone)]
pub enum OrderType {
    Market,
    Limit,
}

pub type PriceMap = HashMap<String, Decimal>;
pub type OrderBookMap = HashMap<String, OrderBook>;

impl Default for TradingFees {
    fn default() -> Self {
        Self {
            maker_fee: Decimal::from_str_exact("0.001").unwrap(), // 0.1%
            taker_fee: Decimal::from_str_exact("0.001").unwrap(), // 0.1%
            withdrawal_fee: Decimal::from_str_exact("0.0005").unwrap(), // 0.05%
        }
    }
}