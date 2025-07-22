use triangular_arbitrage::ArbitrageBot;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    
    let bot = ArbitrageBot::new().await?;
    bot.run().await?;
    
    Ok(())
}