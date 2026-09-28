use rusty_lcu::LiveClientDataClient;

#[tokio::main]
async fn main() -> rusty_lcu::Result<()> {
    let client = LiveClientDataClient::new()?;
    let game = client.all_game_data().await?;

    println!(
        "{} at level {} after {:.1}s",
        game.active_player.summoner_name, game.active_player.level, game.game_data.game_time
    );
    println!("players: {}", game.all_players.len());
    println!("events: {}", game.events.events.len());

    Ok(())
}
