use rusty_lcu::{EndpointParams, LcuClient, generated};

#[tokio::main]
async fn main() -> rusty_lcu::Result<()> {
    let mut client = LcuClient::new()?;
    client.connect().await?;

    let summoner =
        generated::get_lol_summoner_v1_current_summoner_typed(&client, EndpointParams::new())
            .await?;

    println!(
        "{}#{} is level {}",
        summoner.game_name, summoner.tag_line, summoner.summoner_level
    );

    Ok(())
}
