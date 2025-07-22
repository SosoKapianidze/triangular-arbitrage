# Triangular Arbitrage Bot

A Rust-based cryptocurrency arbitrage bot that identifies and executes triangular arbitrage opportunities across Binance and Bybit exchanges.

## ⚠️ Important Disclaimer

**This software is provided for educational purposes only. Cryptocurrency trading involves substantial risk and may result in significant financial losses. Use at your own risk.**

- Always test with small amounts first
- Understand the risks involved in arbitrage trading  
- Monitor your positions closely
- Be aware of exchange fees, slippage, and market volatility

## Features

- **Cross-Exchange Arbitrage**: Detects price differences between Binance and Bybit
- **Triangular Arbitrage**: Identifies opportunities within a single exchange
- **Risk Management**: Configurable position sizes and profit thresholds
- **Real-time Monitoring**: Continuous price monitoring and opportunity detection
- **Secure API Integration**: HMAC-SHA256 signed requests to both exchanges

## Setup

1. **Install Rust** (if not already installed):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Clone and setup the project**:
   ```bash
   git clone <your-repo-url>
   cd triangular-arbitrage
   ```

3. **Configure API credentials**:
   ```bash
   cp .env.example .env
   # Edit .env with your actual API keys
   ```

4. **Build the project**:
   ```bash
   cargo build --release
   ```

5. **Run the bot**:
   ```bash
   RUST_LOG=info cargo run
   ```

## Configuration

Create a `config.json` file to customize the bot's behavior:

```json
{
  "trading": {
    "min_profit_threshold": 0.5,
    "max_position_size": 1000.0,
    "trading_pairs": ["BTCUSDT", "ETHUSDT", "BNBUSDT"],
    "enable_execution": false
  },
  "risk": {
    "max_daily_loss": 100.0,
    "max_open_positions": 3,
    "stop_loss_percentage": 2.0
  },
  "exchanges": {
    "binance_enabled": true,
    "bybit_enabled": true,
    "rate_limit_ms": 100
  }
}
```

## API Permissions

Ensure your API keys have the following permissions:

**Binance**:
- Enable Reading
- Enable Spot & Margin Trading (if executing trades)

**Bybit**:  
- Read access
- Trade access (if executing trades)

## Safety Features

- **Execution Disabled by Default**: The bot only monitors opportunities by default
- **Configurable Thresholds**: Set minimum profit requirements
- **Position Size Limits**: Control maximum trade sizes
- **Rate Limiting**: Respects exchange API limits

## Architecture

```
src/
├── main.rs              # Application entry point
├── lib.rs               # Main bot logic
├── config.rs            # Configuration management
├── arbitrage/           # Arbitrage detection algorithms
│   └── mod.rs
└── exchanges/           # Exchange API clients
    ├── mod.rs
    ├── binance.rs       # Binance API implementation
    └── bybit.rs         # Bybit API implementation
```

## Risk Considerations

1. **Market Risk**: Prices can move against you during execution
2. **Execution Risk**: Network latency and order failures
3. **Liquidity Risk**: Insufficient order book depth
4. **Fee Impact**: Trading fees may eliminate profit margins
5. **API Limits**: Rate limiting and potential downtime

## Testing

Always test with paper trading or very small amounts first:

1. Set `enable_execution: false` in config
2. Monitor detected opportunities
3. Verify profit calculations manually
4. Test with minimal position sizes

## License

This project is licensed under the MIT License - see the LICENSE file for details.

## Contributing

Contributions are welcome! Please:
1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Submit a pull request

## Support

For issues and questions:
- Check the documentation
- Review exchange API documentation
- Test in a sandbox environment first