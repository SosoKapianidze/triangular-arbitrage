use super::{OrderBook, ExchangeError};
use anyhow::Result;
use rust_decimal::Decimal;
use std::collections::HashMap;

pub struct OrderBookAnalyzer;

impl OrderBookAnalyzer {
    pub fn calculate_execution_impact(
        order_book: &OrderBook,
        quantity: Decimal,
        is_buy: bool,
    ) -> Result<OrderBookImpact> {
        let orders = if is_buy { &order_book.asks } else { &order_book.bids };
        
        if orders.is_empty() {
            return Err(ExchangeError::InsufficientBalance(
                "Order book is empty".to_string()
            ).into());
        }
        
        let mut remaining_quantity = quantity;
        let mut total_cost = Decimal::ZERO;
        let mut weighted_avg_price = Decimal::ZERO;
        let mut orders_needed = 0;
        
        for (price, available_qty) in orders {
            if remaining_quantity <= Decimal::ZERO {
                break;
            }
            
            let fill_quantity = remaining_quantity.min(*available_qty);
            total_cost += fill_quantity * price;
            remaining_quantity -= fill_quantity;
            orders_needed += 1;
            
            if remaining_quantity <= Decimal::ZERO {
                break;
            }
        }
        
        if remaining_quantity > Decimal::ZERO {
            return Err(ExchangeError::InsufficientBalance(format!(
                "Insufficient liquidity. Need {} more units",
                remaining_quantity
            )).into());
        }
        
        weighted_avg_price = total_cost / quantity;
        
        // Calculate slippage compared to best price
        let best_price = orders[0].0;
        let slippage = ((weighted_avg_price - best_price) / best_price).abs() * Decimal::ONE_HUNDRED;
        
        Ok(OrderBookImpact {
            weighted_avg_price,
            total_cost,
            slippage_percentage: slippage,
            orders_needed,
            is_executable: true,
        })
    }
    
    pub fn check_minimum_liquidity(
        order_book: &OrderBook,
        min_depth_usd: Decimal,
    ) -> bool {
        let bid_depth = Self::calculate_depth(&order_book.bids);
        let ask_depth = Self::calculate_depth(&order_book.asks);
        
        bid_depth >= min_depth_usd && ask_depth >= min_depth_usd
    }
    
    fn calculate_depth(orders: &[(Decimal, Decimal)]) -> Decimal {
        orders.iter()
            .take(10) // Top 10 orders
            .map(|(price, quantity)| price * quantity)
            .sum()
    }
    
    pub fn estimate_execution_time(orders_needed: usize) -> std::time::Duration {
        // Estimate based on typical exchange latency
        let base_latency = std::time::Duration::from_millis(100);
        let per_order_latency = std::time::Duration::from_millis(50);
        
        base_latency + per_order_latency * orders_needed as u32
    }
}

#[derive(Debug, Clone)]
pub struct OrderBookImpact {
    pub weighted_avg_price: Decimal,
    pub total_cost: Decimal,
    pub slippage_percentage: Decimal,
    pub orders_needed: usize,
    pub is_executable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    
    #[test]
    fn test_order_book_impact_calculation() {
        let order_book = OrderBook {
            symbol: "BTCUSDT".to_string(),
            bids: vec![
                (Decimal::from_str_exact("50000.0").unwrap(), Decimal::from_str_exact("1.0").unwrap()),
                (Decimal::from_str_exact("49990.0").unwrap(), Decimal::from_str_exact("2.0").unwrap()),
            ],
            asks: vec![
                (Decimal::from_str_exact("50010.0").unwrap(), Decimal::from_str_exact("1.0").unwrap()),
                (Decimal::from_str_exact("50020.0").unwrap(), Decimal::from_str_exact("2.0").unwrap()),
            ],
            timestamp: Utc::now(),
        };
        
        let impact = OrderBookAnalyzer::calculate_execution_impact(
            &order_book,
            Decimal::from_str_exact("1.5").unwrap(),
            true, // buy order
        ).unwrap();
        
        assert!(impact.is_executable);
        assert_eq!(impact.orders_needed, 2);
        assert!(impact.slippage_percentage > Decimal::ZERO);
    }
    
    #[test]
    fn test_insufficient_liquidity() {
        let order_book = OrderBook {
            symbol: "BTCUSDT".to_string(),
            bids: vec![],
            asks: vec![
                (Decimal::from_str_exact("50010.0").unwrap(), Decimal::from_str_exact("0.5").unwrap()),
            ],
            timestamp: Utc::now(),
        };
        
        let result = OrderBookAnalyzer::calculate_execution_impact(
            &order_book,
            Decimal::from_str_exact("1.0").unwrap(),
            true,
        );
        
        assert!(result.is_err());
    }
}