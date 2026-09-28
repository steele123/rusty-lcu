//! Typed access to Riot's Live Client Data API.
//!
//! The API is available only while a League game is running and does not use
//! LCU credentials. Riot serves it locally over HTTPS with a self-signed
//! certificate at `https://127.0.0.1:2999/liveclientdata`.
//!
//! # Availability
//!
//! [`LiveClientDataClient::is_game_running`] performs a single availability
//! check. [`LiveClientDataClient::wait_for_game`] polls until a game starts and
//! supports both a deadline and [`CancellationToken`].
//!
//! # Events
//!
//! Riot exposes a cumulative event list rather than a push connection.
//! [`LiveEventStream`] polls that list and emits each observed entry once. Use
//! [`GameEvent::kind`] for typed access to common event payloads while retaining
//! the raw fields in [`GameEvent::data`].

#![warn(missing_docs)]

use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

use reqwest::Url;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::time::{Instant, sleep, sleep_until};
pub use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

/// Default origin exposed by the running League game process.
pub const DEFAULT_BASE_URL: &str = "https://127.0.0.1:2999/";

/// Async client for Riot's local Live Client Data API.
///
/// Construct this with [`LiveClientDataClient::new`] for normal use or
/// [`LiveClientDataClient::builder`] to configure timeouts and retries.
#[derive(Debug, Clone)]
pub struct LiveClientDataClient {
    http: reqwest::Client,
    base_url: Url,
    max_retries: usize,
    retry_delay: Duration,
}

/// Configures and constructs a [`LiveClientDataClient`].
///
/// Defaults to a five-second request timeout, no retries, and a 250 ms retry
/// delay. Retries apply to transport failures and HTTP 502, 503, and 504.
#[derive(Debug, Clone)]
pub struct LiveClientDataClientBuilder {
    base_url: String,
    request_timeout: Duration,
    max_retries: usize,
    retry_delay: Duration,
}

impl Default for LiveClientDataClientBuilder {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            request_timeout: Duration::from_secs(5),
            max_retries: 0,
            retry_delay: Duration::from_millis(250),
        }
    }
}

impl LiveClientDataClientBuilder {
    /// Replaces the game-client origin.
    ///
    /// The client always appends `/liveclientdata/` to this origin.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Sets the timeout applied to each individual HTTP request.
    pub fn request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// Sets the maximum number of attempts after the initial request.
    pub fn max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Sets the delay between retry attempts.
    pub fn retry_delay(mut self, retry_delay: Duration) -> Self {
        self.retry_delay = retry_delay;
        self
    }

    /// Builds the configured client.
    pub fn build(self) -> Result<LiveClientDataClient> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(self.request_timeout)
            .build()?;
        let mut base_url = Url::parse(&self.base_url)?;
        base_url.set_path("/liveclientdata/");
        base_url.set_query(None);
        base_url.set_fragment(None);

        Ok(LiveClientDataClient {
            http,
            base_url,
            max_retries: self.max_retries,
            retry_delay: self.retry_delay,
        })
    }
}

/// Controls [`LiveClientDataClient::wait_for_game`].
///
/// The default polls once per second, has no overall timeout, and uses a fresh
/// cancellation token.
#[derive(Debug, Clone)]
pub struct WaitForGameOptions {
    /// Delay between unavailable-game checks.
    pub poll_interval: Duration,
    /// Overall deadline, or `None` to wait indefinitely.
    pub timeout: Option<Duration>,
    /// Token that interrupts polling and in-flight requests when cancelled.
    pub cancellation_token: CancellationToken,
}

impl Default for WaitForGameOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(1),
            timeout: None,
            cancellation_token: CancellationToken::new(),
        }
    }
}

/// Controls polling performed by [`LiveEventStream`].
///
/// The default polls every 500 ms and ignores events already present in the
/// first response.
#[derive(Debug, Clone)]
pub struct LiveEventPollOptions {
    /// Delay between event-list requests when no new event is available.
    pub poll_interval: Duration,
    /// Whether events from the first API response should be emitted.
    pub include_existing_events: bool,
    /// Token that interrupts sleeps and in-flight requests when cancelled.
    pub cancellation_token: CancellationToken,
}

impl Default for LiveEventPollOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(500),
            include_existing_events: false,
            cancellation_token: CancellationToken::new(),
        }
    }
}

/// Stateful polling adapter for Riot's cumulative event list.
///
/// Events are tracked by their position in the cumulative list instead of
/// `EventID`, because Riot's sample payloads do not guarantee unique IDs. A
/// shorter list is treated as a new game and resets the cursor.
pub struct LiveEventStream {
    client: LiveClientDataClient,
    options: LiveEventPollOptions,
    initialized: bool,
    next_index: usize,
    pending: VecDeque<GameEvent>,
}

impl LiveClientDataClient {
    /// Creates a client for [`DEFAULT_BASE_URL`] with default request settings.
    pub fn new() -> Result<Self> {
        Self::builder().build()
    }

    /// Starts configuring a client.
    pub fn builder() -> LiveClientDataClientBuilder {
        LiveClientDataClientBuilder::default()
    }

    /// Creates a client using a custom game-client origin.
    ///
    /// This is primarily useful for tests and proxies. The
    /// `/liveclientdata/` path is appended automatically.
    pub fn with_base_url(base_url: impl AsRef<str>) -> Result<Self> {
        Self::builder().base_url(base_url.as_ref()).build()
    }

    /// Requests any Live Client Data endpoint and deserializes its response.
    ///
    /// `endpoint` is relative to `/liveclientdata/`, for example `gamestats`.
    pub async fn get<T>(&self, endpoint: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.get_with_query(endpoint, &[]).await
    }

    /// Returns the complete live-game snapshot from `allgamedata`.
    ///
    /// Prefer a narrower endpoint when the application does not need every
    /// section of the response.
    pub async fn all_game_data(&self) -> Result<AllGameData> {
        self.get("allgamedata").await
    }

    /// Returns abilities, stats, gold, runes, and identity for the local player.
    pub async fn active_player(&self) -> Result<ActivePlayer> {
        self.get("activeplayer").await
    }

    /// Returns the local player's Riot ID string.
    pub async fn active_player_name(&self) -> Result<String> {
        self.get("activeplayername").await
    }

    /// Returns the local player's abilities keyed by slot, such as `Q` or `Passive`.
    pub async fn active_player_abilities(&self) -> Result<Abilities> {
        self.get("activeplayerabilities").await
    }

    /// Returns the local player's complete rune selection.
    pub async fn active_player_runes(&self) -> Result<FullRunes> {
        self.get("activeplayerrunes").await
    }

    /// Returns every player currently present in the match.
    pub async fn player_list(&self) -> Result<Vec<Player>> {
        self.get("playerlist").await
    }

    /// Returns scoreboard values for the player identified by `riot_id`.
    pub async fn player_scores(&self, riot_id: &str) -> Result<PlayerScores> {
        self.get_for_player("playerscores", riot_id).await
    }

    /// Returns summoner spells for the player identified by `riot_id`.
    pub async fn player_summoner_spells(&self, riot_id: &str) -> Result<SummonerSpells> {
        self.get_for_player("playersummonerspells", riot_id).await
    }

    /// Returns primary rune information for the player identified by `riot_id`.
    pub async fn player_main_runes(&self, riot_id: &str) -> Result<MainRunes> {
        self.get_for_player("playermainrunes", riot_id).await
    }

    /// Returns inventory items for the player identified by `riot_id`.
    pub async fn player_items(&self, riot_id: &str) -> Result<Vec<Item>> {
        self.get_for_player("playeritems", riot_id).await
    }

    /// Returns the cumulative list of events observed during this game.
    pub async fn event_data(&self) -> Result<EventData> {
        self.get("eventdata").await
    }

    /// Returns basic map, mode, and game-clock information.
    pub async fn game_stats(&self) -> Result<GameStats> {
        self.get("gamestats").await
    }

    /// Returns whether the local game API is currently reachable.
    ///
    /// Connection failures, request timeouts, HTTP 404, and HTTP 503 indicate
    /// that no game is available. Other failures remain errors.
    pub async fn is_game_running(&self) -> Result<bool> {
        match self.game_stats().await {
            Ok(_) => Ok(true),
            Err(error) if is_unavailable_error(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Waits until a game is available, the timeout expires, or cancellation is requested.
    ///
    /// The overall timeout and cancellation token interrupt both polling sleeps
    /// and in-flight HTTP requests. A timeout returns
    /// [`Error::LiveClientWaitTimedOut`]; cancellation returns
    /// [`Error::OperationCancelled`].
    pub async fn wait_for_game(&self, options: WaitForGameOptions) -> Result<GameStats> {
        let started = Instant::now();
        let deadline = options.timeout.map(|timeout| started + timeout);

        loop {
            let request = self.game_stats();
            tokio::pin!(request);

            let result = if let Some(deadline) = deadline {
                tokio::select! {
                    _ = options.cancellation_token.cancelled() => {
                        return Err(Error::OperationCancelled);
                    }
                    _ = sleep_until(deadline) => {
                        return Err(Error::LiveClientWaitTimedOut {
                            timeout: options.timeout.expect("deadline has timeout"),
                        });
                    }
                    result = &mut request => result,
                }
            } else {
                tokio::select! {
                    _ = options.cancellation_token.cancelled() => {
                        return Err(Error::OperationCancelled);
                    }
                    result = &mut request => result,
                }
            };

            match result {
                Ok(stats) => return Ok(stats),
                Err(error) if is_unavailable_error(&error) => {}
                Err(error) => return Err(error),
            }

            if let Some(deadline) = deadline {
                if Instant::now() >= deadline {
                    return Err(Error::LiveClientWaitTimedOut {
                        timeout: options.timeout.expect("deadline has timeout"),
                    });
                }

                tokio::select! {
                    _ = options.cancellation_token.cancelled() => {
                        return Err(Error::OperationCancelled);
                    }
                    _ = sleep(options.poll_interval) => {}
                    _ = sleep_until(deadline) => {
                        return Err(Error::LiveClientWaitTimedOut {
                            timeout: options.timeout.expect("deadline has timeout"),
                        });
                    }
                }
            } else {
                tokio::select! {
                    _ = options.cancellation_token.cancelled() => {
                        return Err(Error::OperationCancelled);
                    }
                    _ = sleep(options.poll_interval) => {}
                }
            }
        }
    }

    /// Creates a polling stream that yields each observed in-game event once.
    ///
    /// No background task is created. Requests are made when
    /// [`LiveEventStream::next_event`] is awaited.
    pub fn event_stream(&self, options: LiveEventPollOptions) -> LiveEventStream {
        LiveEventStream {
            client: self.clone(),
            options,
            initialized: false,
            next_index: 0,
            pending: VecDeque::new(),
        }
    }

    async fn get_for_player<T>(&self, endpoint: &str, riot_id: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.get_with_query(endpoint, &[("riotId", riot_id)]).await
    }

    async fn get_with_query<T>(&self, endpoint: &str, query: &[(&str, &str)]) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let mut url = self.base_url.join(endpoint.trim_start_matches('/'))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }

        for attempt in 0..=self.max_retries {
            match self.http.get(url.clone()).send().await {
                Ok(response) => {
                    let status = response.status();
                    let bytes = response.bytes().await?;

                    if status.is_success() {
                        return Ok(serde_json::from_slice(&bytes)?);
                    }

                    if attempt < self.max_retries && is_retryable_status(status) {
                        sleep(self.retry_delay).await;
                        continue;
                    }

                    return Err(Error::LiveClientData {
                        status,
                        body: String::from_utf8_lossy(&bytes).into_owned(),
                    });
                }
                Err(error) if attempt < self.max_retries => {
                    sleep(self.retry_delay).await;
                    drop(error);
                }
                Err(error) => return Err(error.into()),
            }
        }

        unreachable!("request loop always returns")
    }
}

impl LiveEventStream {
    /// Returns a clone of the token controlling this stream.
    ///
    /// The clone can be moved to another task and cancelled from there.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.options.cancellation_token.clone()
    }

    /// Cancels this stream.
    ///
    /// A pending or future [`next_event`](Self::next_event) call returns
    /// `Ok(None)` after observing cancellation.
    pub fn cancel(&self) {
        self.options.cancellation_token.cancel();
    }

    /// Returns the next unseen event, or `None` after cancellation.
    ///
    /// API and deserialization errors are returned immediately. Calling this
    /// method again after an error retries from the same event cursor.
    pub async fn next_event(&mut self) -> Result<Option<GameEvent>> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }

            if self.options.cancellation_token.is_cancelled() {
                return Ok(None);
            }

            let request = self.client.event_data();
            tokio::pin!(request);
            let events = tokio::select! {
                _ = self.options.cancellation_token.cancelled() => return Ok(None),
                result = &mut request => result?.events,
            };

            if !self.initialized {
                self.next_index = if self.options.include_existing_events {
                    0
                } else {
                    events.len()
                };
                self.initialized = true;
            } else if events.len() < self.next_index {
                // The cumulative event list resets when a new game begins.
                self.next_index = 0;
            }

            self.pending
                .extend(events[self.next_index..].iter().cloned());
            self.next_index = events.len();

            if let Some(event) = self.pending.pop_front() {
                return Ok(Some(event));
            }

            tokio::select! {
                _ = self.options.cancellation_token.cancelled() => return Ok(None),
                _ = sleep(self.options.poll_interval) => {}
            }
        }
    }
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::BAD_GATEWAY
            | reqwest::StatusCode::SERVICE_UNAVAILABLE
            | reqwest::StatusCode::GATEWAY_TIMEOUT
    )
}

fn is_unavailable_error(error: &Error) -> bool {
    match error {
        Error::Request(error) => error.is_connect() || error.is_timeout(),
        Error::LiveClientData { status, .. } => matches!(
            *status,
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::SERVICE_UNAVAILABLE
        ),
        _ => false,
    }
}

/// Complete response returned by the `allgamedata` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AllGameData {
    /// Data for the player running this client.
    pub active_player: ActivePlayer,
    /// All players in the current match.
    pub all_players: Vec<Player>,
    /// Events accumulated since the match began.
    pub events: EventData,
    /// Current map, mode, and clock information.
    pub game_data: GameStats,
}

/// Abilities keyed by Riot's slot name, such as `Q`, `W`, or `Passive`.
pub type Abilities = BTreeMap<String, Ability>;

/// One active-player ability.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Ability {
    /// Current rank, absent for abilities such as champion passives.
    #[serde(default)]
    pub ability_level: Option<u32>,
    /// Localized display name.
    pub display_name: String,
    /// Riot's internal ability identifier.
    pub id: String,
    /// Localization key for the description.
    pub raw_description: String,
    /// Localization key for the display name.
    pub raw_display_name: String,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Detailed state for the player running the local game client.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivePlayer {
    /// Abilities keyed by slot name.
    pub abilities: Abilities,
    /// Current combat and resource statistics.
    pub champion_stats: ChampionStats,
    /// Gold currently available to spend.
    pub current_gold: f64,
    /// Complete selected rune page.
    pub full_runes: FullRunes,
    /// Current champion level.
    pub level: u32,
    /// Legacy-compatible player name returned by Riot.
    pub summoner_name: String,
    /// Full Riot ID, when provided by this game build.
    #[serde(default)]
    pub riot_id: Option<String>,
    /// Game-name portion of the Riot ID, when provided.
    #[serde(default)]
    pub riot_id_game_name: Option<String>,
    /// Tag-line portion of the Riot ID, when provided.
    #[serde(default)]
    pub riot_id_tag_line: Option<String>,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Current combat, movement, health, and resource values for the active champion.
///
/// Missing numeric fields default to zero because Riot's published sample and
/// current response schema differ. Unknown fields remain available in
/// [`extra`](Self::extra).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ChampionStats {
    /// Ability haste.
    pub ability_haste: f64,
    /// Ability power.
    pub ability_power: f64,
    /// Armor after current modifiers.
    pub armor: f64,
    /// Flat armor penetration.
    pub armor_penetration_flat: f64,
    /// Percentage armor penetration.
    pub armor_penetration_percent: f64,
    /// Attack damage after current modifiers.
    pub attack_damage: f64,
    /// Basic attack range.
    pub attack_range: f64,
    /// Attacks per second.
    pub attack_speed: f64,
    /// Bonus percentage armor penetration.
    pub bonus_armor_penetration_percent: f64,
    /// Bonus percentage magic penetration.
    pub bonus_magic_penetration_percent: f64,
    /// Legacy cooldown-reduction value.
    pub cooldown_reduction: f64,
    /// Critical-strike chance as a fraction.
    pub crit_chance: f64,
    /// Critical-strike damage multiplier.
    pub crit_damage: f64,
    /// Current health.
    pub current_health: f64,
    /// Health regenerated per second.
    pub health_regen_rate: f64,
    /// Life-steal value as a fraction.
    pub life_steal: f64,
    /// Flat magic lethality.
    pub magic_lethality: f64,
    /// Flat magic penetration.
    pub magic_penetration_flat: f64,
    /// Percentage magic penetration.
    pub magic_penetration_percent: f64,
    /// Magic resistance after current modifiers.
    pub magic_resist: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Current movement speed.
    pub move_speed: f64,
    /// Flat physical lethality.
    pub physical_lethality: f64,
    /// Maximum value of the champion's current resource.
    pub resource_max: f64,
    /// Resource regenerated per second.
    pub resource_regen_rate: f64,
    /// Resource kind, such as `MANA` or `ENERGY`.
    pub resource_type: String,
    /// Current resource value.
    pub resource_value: f64,
    /// Spell-vamp value as a fraction.
    pub spell_vamp: f64,
    /// Crowd-control duration reduction as a fraction.
    pub tenacity: f64,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A selected rune or rune-tree descriptor.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Rune {
    /// Riot's numeric rune or tree identifier.
    pub id: u32,
    /// Localized name, absent for some stat shards.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Localization key for the description.
    pub raw_description: String,
    /// Localization key for the display name, when present.
    #[serde(default)]
    pub raw_display_name: Option<String>,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Keystone and primary/secondary rune-tree choices for a player.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MainRunes {
    /// Selected keystone rune.
    pub keystone: Rune,
    /// Selected primary rune tree.
    pub primary_rune_tree: Rune,
    /// Selected secondary rune tree.
    pub secondary_rune_tree: Rune,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Complete rune selection for the active player.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FullRunes {
    /// All selected non-stat runes, including the keystone.
    pub general_runes: Vec<Rune>,
    /// Selected keystone rune.
    pub keystone: Rune,
    /// Selected primary rune tree.
    pub primary_rune_tree: Rune,
    /// Selected secondary rune tree.
    pub secondary_rune_tree: Rune,
    /// Selected offensive, flexible, and defensive stat shards.
    pub stat_runes: Vec<Rune>,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Public live-game information for one participant.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    /// Localized champion name.
    pub champion_name: String,
    /// Whether Riot identifies the participant as a bot.
    pub is_bot: bool,
    /// Whether the champion is currently dead.
    pub is_dead: bool,
    /// Current inventory.
    pub items: Vec<Item>,
    /// Current champion level.
    pub level: u32,
    /// Assigned or detected position, when available.
    pub position: String,
    /// Localization key for the champion name.
    pub raw_champion_name: String,
    /// Remaining respawn time in seconds.
    pub respawn_timer: f64,
    /// Keystone and selected rune trees.
    pub runes: MainRunes,
    /// Current scoreboard values.
    pub scores: PlayerScores,
    /// Numeric champion skin identifier.
    #[serde(rename = "skinID")]
    pub skin_id: u32,
    /// Legacy-compatible player name returned by Riot.
    pub summoner_name: String,
    /// Full Riot ID, when provided by this game build.
    #[serde(default)]
    pub riot_id: Option<String>,
    /// Game-name portion of the Riot ID, when provided.
    #[serde(default)]
    pub riot_id_game_name: Option<String>,
    /// Tag-line portion of the Riot ID, when provided.
    #[serde(default)]
    pub riot_id_tag_line: Option<String>,
    /// Equipped summoner spells.
    pub summoner_spells: SummonerSpells,
    /// Riot team identifier, commonly `ORDER` or `CHAOS`.
    pub team: String,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Current scoreboard values for one player.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlayerScores {
    /// Champion-kill assists.
    pub assists: u32,
    /// Minions and monsters killed.
    pub creep_score: u32,
    /// Champion deaths.
    pub deaths: u32,
    /// Champion kills.
    pub kills: u32,
    /// Vision contribution score.
    pub ward_score: f64,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Both summoner spells equipped by a player.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummonerSpells {
    /// First summoner-spell slot.
    pub summoner_spell_one: SummonerSpell,
    /// Second summoner-spell slot.
    pub summoner_spell_two: SummonerSpell,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Display and localization information for a summoner spell.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SummonerSpell {
    /// Localized display name.
    pub display_name: String,
    /// Localization key for the description.
    pub raw_description: String,
    /// Localization key for the display name.
    pub raw_display_name: String,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// One item in a player's inventory.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// Whether the item can currently be activated.
    pub can_use: bool,
    /// Whether using the item consumes a charge or the item itself.
    pub consumable: bool,
    /// Stack or charge count.
    pub count: u32,
    /// Localized display name.
    pub display_name: String,
    /// Riot's numeric item identifier.
    #[serde(rename = "itemID")]
    pub item_id: u32,
    /// Current shop price reported by the game.
    pub price: u32,
    /// Localization key for the description.
    pub raw_description: String,
    /// Localization key for the display name.
    pub raw_display_name: String,
    /// Zero-based inventory slot.
    pub slot: u32,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Wrapper returned by the `eventdata` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct EventData {
    /// Events accumulated since the current game began.
    #[serde(rename = "Events")]
    pub events: Vec<GameEvent>,
}

/// One entry from Riot's cumulative live-game event list.
///
/// The common ID, name, and timestamp fields are modeled directly. Event-type
/// specific values remain in [`data`](Self::data) and can be interpreted with
/// [`kind`](Self::kind).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GameEvent {
    /// Event sequence identifier supplied by Riot.
    #[serde(rename = "EventID")]
    pub event_id: u64,
    /// Riot event discriminator, such as `ChampionKill`.
    #[serde(rename = "EventName")]
    pub event_name: String,
    /// Game-clock timestamp in seconds.
    #[serde(rename = "EventTime")]
    pub event_time: f64,
    /// Event-specific fields not shared by every event type.
    #[serde(flatten)]
    pub data: BTreeMap<String, Value>,
}

impl GameEvent {
    /// Interprets the event-specific fields while preserving the raw payload on `self`.
    pub fn kind(&self) -> GameEventKind {
        match self.event_name.as_str() {
            "GameStart" => GameEventKind::GameStart,
            "MinionsSpawning" => GameEventKind::MinionsSpawning,
            "GameEnd" => GameEventKind::GameEnd {
                result: self.data_string("Result"),
            },
            "ChampionKill" => GameEventKind::ChampionKill(ChampionKillEvent {
                killer_name: self.data_string("KillerName"),
                victim_name: self.data_string("VictimName"),
                assisters: self.data_strings("Assisters"),
            }),
            "DragonKill" => GameEventKind::DragonKill(ObjectiveKillEvent {
                killer_name: self.data_string("KillerName"),
                assisters: self.data_strings("Assisters"),
                stolen: self.data_bool("Stolen"),
                objective_type: self.data_string("DragonType"),
            }),
            "HeraldKill" => GameEventKind::HeraldKill(ObjectiveKillEvent {
                killer_name: self.data_string("KillerName"),
                assisters: self.data_strings("Assisters"),
                stolen: self.data_bool("Stolen"),
                objective_type: None,
            }),
            "BaronKill" => GameEventKind::BaronKill(ObjectiveKillEvent {
                killer_name: self.data_string("KillerName"),
                assisters: self.data_strings("Assisters"),
                stolen: self.data_bool("Stolen"),
                objective_type: None,
            }),
            "TurretKilled" => GameEventKind::TurretKilled(StructureKillEvent {
                killer_name: self.data_string("KillerName"),
                assisters: self.data_strings("Assisters"),
                structure_name: self.data_string("TurretKilled"),
            }),
            "InhibKilled" => GameEventKind::InhibKilled(StructureKillEvent {
                killer_name: self.data_string("KillerName"),
                assisters: self.data_strings("Assisters"),
                structure_name: self.data_string("InhibKilled"),
            }),
            "FirstBrick" => GameEventKind::FirstBrick {
                killer_name: self.data_string("KillerName"),
            },
            "Multikill" => GameEventKind::Multikill(MultikillEvent {
                killer_name: self.data_string("KillerName"),
                kill_streak: self
                    .data
                    .get("KillStreak")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok()),
            }),
            "Ace" => GameEventKind::Ace(AceEvent {
                acer: self.data_string("Acer"),
                acing_team: self.data_string("AcingTeam"),
            }),
            _ => GameEventKind::Other {
                name: self.event_name.clone(),
            },
        }
    }

    fn data_string(&self, key: &str) -> Option<String> {
        self.data
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    fn data_strings(&self, key: &str) -> Vec<String> {
        self.data
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    }

    fn data_bool(&self, key: &str) -> Option<bool> {
        self.data.get(key).and_then(|value| match value {
            Value::Bool(value) => Some(*value),
            Value::String(value) if value.eq_ignore_ascii_case("true") => Some(true),
            Value::String(value) if value.eq_ignore_ascii_case("false") => Some(false),
            _ => None,
        })
    }
}

/// Typed interpretation of common [`GameEvent`] payloads.
///
/// Unknown event names are preserved in [`Other`](Self::Other), and every raw
/// field remains available on the original [`GameEvent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameEventKind {
    /// The game session started.
    GameStart,
    /// Lane minions began spawning.
    MinionsSpawning,
    /// The game ended with Riot's optional result string.
    GameEnd {
        /// Riot's result value, when supplied.
        result: Option<String>,
    },
    /// A champion killed another champion.
    ChampionKill(ChampionKillEvent),
    /// A team killed a dragon.
    DragonKill(ObjectiveKillEvent),
    /// A team killed the Rift Herald.
    HeraldKill(ObjectiveKillEvent),
    /// A team killed Baron Nashor.
    BaronKill(ObjectiveKillEvent),
    /// A turret was destroyed.
    TurretKilled(StructureKillEvent),
    /// An inhibitor was destroyed.
    InhibKilled(StructureKillEvent),
    /// The first turret of the game was destroyed.
    FirstBrick {
        /// Riot ID or legacy name credited with the first turret.
        killer_name: Option<String>,
    },
    /// One player earned a multikill.
    Multikill(MultikillEvent),
    /// A team was aced.
    Ace(AceEvent),
    /// An event this crate does not currently interpret.
    Other {
        /// Unrecognized Riot event name.
        name: String,
    },
}

/// Fields associated with a `ChampionKill` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChampionKillEvent {
    /// Riot ID or legacy name of the killer, when supplied.
    pub killer_name: Option<String>,
    /// Riot ID or legacy name of the victim, when supplied.
    pub victim_name: Option<String>,
    /// Riot IDs or legacy names credited with assists.
    pub assisters: Vec<String>,
}

/// Shared fields for dragon, Herald, and Baron kill events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectiveKillEvent {
    /// Riot ID or legacy name of the killer, when supplied.
    pub killer_name: Option<String>,
    /// Riot IDs or legacy names credited with assists.
    pub assisters: Vec<String>,
    /// Whether the objective was stolen, when supplied.
    pub stolen: Option<bool>,
    /// Objective subtype, such as a dragon element, when supplied.
    pub objective_type: Option<String>,
}

/// Shared fields for turret and inhibitor destruction events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureKillEvent {
    /// Riot ID or legacy name of the killer, when supplied.
    pub killer_name: Option<String>,
    /// Riot IDs or legacy names credited with assists.
    pub assisters: Vec<String>,
    /// Riot's internal name for the destroyed structure.
    pub structure_name: Option<String>,
}

/// Fields associated with a `Multikill` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultikillEvent {
    /// Riot ID or legacy name of the player, when supplied.
    pub killer_name: Option<String>,
    /// Number of kills in the streak.
    pub kill_streak: Option<u32>,
}

/// Fields associated with an `Ace` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AceEvent {
    /// Player credited by Riot as the acer, when supplied.
    pub acer: Option<String>,
    /// Team that scored the ace, when supplied.
    pub acing_team: Option<String>,
}

/// Basic state returned by the `gamestats` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameStats {
    /// Riot game-mode identifier, such as `CLASSIC`.
    pub game_mode: String,
    /// Current game clock in seconds.
    pub game_time: f64,
    /// Riot map name, such as `Map11`.
    pub map_name: String,
    /// Numeric Riot map identifier.
    pub map_number: u32,
    /// Current terrain variant.
    pub map_terrain: String,
    /// Fields added by Riot that this crate does not yet model explicitly.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::*;

    fn serve_once(response_status: &str, body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_owned();
        let status = response_status.to_owned();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let bytes_read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..bytes_read]).into_owned();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        });

        (format!("http://{address}"), handle)
    }

    fn serve_responses(responses: Vec<(&str, &str)>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let responses = responses
            .into_iter()
            .map(|(status, body)| (status.to_owned(), body.to_owned()))
            .collect::<Vec<_>>();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0; 4096];
                let bytes_read = stream.read(&mut buffer).unwrap();
                requests.push(String::from_utf8_lossy(&buffer[..bytes_read]).into_owned());
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            requests
        });

        (format!("http://{address}"), handle)
    }

    fn serve_stalled() -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _bytes_read = stream.read(&mut buffer).unwrap();
            thread::sleep(Duration::from_millis(100));
        });

        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn requests_and_decodes_game_stats() {
        let (base_url, server) = serve_once(
            "200 OK",
            r#"{"gameMode":"CLASSIC","gameTime":42.5,"mapName":"Map11","mapNumber":11,"mapTerrain":"Default"}"#,
        );
        let client = LiveClientDataClient::with_base_url(base_url).unwrap();

        let stats = client.game_stats().await.unwrap();
        let request = server.join().unwrap();

        assert_eq!(stats.game_mode, "CLASSIC");
        assert_eq!(stats.game_time, 42.5);
        assert!(request.starts_with("GET /liveclientdata/gamestats HTTP/1.1"));
    }

    #[tokio::test]
    async fn encodes_riot_id_query_parameter() {
        let (base_url, server) = serve_once(
            "200 OK",
            r#"{"assists":1,"creepScore":12,"deaths":2,"kills":3,"wardScore":4.5}"#,
        );
        let client = LiveClientDataClient::with_base_url(base_url).unwrap();

        let scores = client.player_scores("Riot Tuxedo#NA 1").await.unwrap();
        let request = server.join().unwrap();

        assert_eq!(scores.kills, 3);
        assert!(request.contains("GET /liveclientdata/playerscores?riotId=Riot+Tuxedo%23NA+1 "));
    }

    #[tokio::test]
    async fn reports_unsuccessful_responses() {
        let (base_url, server) = serve_once("404 Not Found", r#"{"error":"not in game"}"#);
        let client = LiveClientDataClient::with_base_url(base_url).unwrap();

        let error = client.game_stats().await.unwrap_err();
        server.join().unwrap();

        assert!(matches!(
            error,
            Error::LiveClientData {
                status: reqwest::StatusCode::NOT_FOUND,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn retries_transient_responses() {
        let (base_url, server) = serve_responses(vec![
            ("503 Service Unavailable", r#"{"error":"starting"}"#),
            (
                "200 OK",
                r#"{"gameMode":"CLASSIC","gameTime":1.0,"mapName":"Map11","mapNumber":11,"mapTerrain":"Default"}"#,
            ),
        ]);
        let client = LiveClientDataClient::builder()
            .base_url(base_url)
            .max_retries(1)
            .retry_delay(Duration::ZERO)
            .build()
            .unwrap();

        assert!(client.is_game_running().await.unwrap());
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn waits_for_game_to_become_available() {
        let (base_url, server) = serve_responses(vec![
            ("503 Service Unavailable", r#"{"error":"not in game"}"#),
            (
                "200 OK",
                r#"{"gameMode":"CLASSIC","gameTime":2.0,"mapName":"Map11","mapNumber":11,"mapTerrain":"Default"}"#,
            ),
        ]);
        let client = LiveClientDataClient::with_base_url(base_url).unwrap();

        let stats = client
            .wait_for_game(WaitForGameOptions {
                poll_interval: Duration::ZERO,
                timeout: Some(Duration::from_secs(1)),
                cancellation_token: CancellationToken::new(),
            })
            .await
            .unwrap();

        assert_eq!(stats.game_time, 2.0);
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn wait_for_game_cancels_an_in_flight_request() {
        let (base_url, server) = serve_stalled();
        let client = LiveClientDataClient::with_base_url(base_url).unwrap();
        let cancellation_token = CancellationToken::new();
        let cancel = cancellation_token.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(10)).await;
            cancel.cancel();
        });

        let error = client
            .wait_for_game(WaitForGameOptions {
                cancellation_token,
                ..WaitForGameOptions::default()
            })
            .await
            .unwrap_err();

        assert!(matches!(error, Error::OperationCancelled));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn event_stream_yields_only_new_events() {
        let (base_url, server) = serve_responses(vec![
            (
                "200 OK",
                r#"{"Events":[{"EventID":0,"EventName":"GameStart","EventTime":0.0}]}"#,
            ),
            (
                "200 OK",
                r#"{"Events":[{"EventID":0,"EventName":"GameStart","EventTime":0.0},{"EventID":1,"EventName":"ChampionKill","EventTime":12.5,"KillerName":"Ahri","VictimName":"Lux"}]}"#,
            ),
        ]);
        let client = LiveClientDataClient::with_base_url(base_url).unwrap();
        let mut events = client.event_stream(LiveEventPollOptions {
            poll_interval: Duration::ZERO,
            ..LiveEventPollOptions::default()
        });

        let event = events.next_event().await.unwrap().unwrap();

        assert_eq!(event.event_id, 1);
        assert!(matches!(event.kind(), GameEventKind::ChampionKill(_)));
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn event_stream_stops_when_cancelled() {
        let client = LiveClientDataClient::new().unwrap();
        let mut events = client.event_stream(LiveEventPollOptions::default());
        events.cancel();

        assert!(events.next_event().await.unwrap().is_none());
    }

    #[test]
    fn decodes_variable_event_fields() {
        let events: EventData = serde_json::from_str(
            r#"{"Events":[{"EventID":7,"EventName":"ChampionKill","EventTime":12.5,"KillerName":"Ahri","VictimName":"Lux"}]}"#,
        )
        .unwrap();

        assert_eq!(events.events[0].event_name, "ChampionKill");
        assert_eq!(events.events[0].data["KillerName"], "Ahri");
        assert_eq!(
            events.events[0].kind(),
            GameEventKind::ChampionKill(ChampionKillEvent {
                killer_name: Some("Ahri".to_string()),
                victim_name: Some("Lux".to_string()),
                assisters: Vec::new(),
            })
        );
    }

    #[test]
    fn interprets_objective_event_fields() {
        let event: GameEvent = serde_json::from_str(
            r#"{"EventID":8,"EventName":"DragonKill","EventTime":20.0,"KillerName":"Ahri","Assisters":["Lux"],"DragonType":"Earth","Stolen":"False"}"#,
        )
        .unwrap();

        assert_eq!(
            event.kind(),
            GameEventKind::DragonKill(ObjectiveKillEvent {
                killer_name: Some("Ahri".to_string()),
                assisters: vec!["Lux".to_string()],
                stolen: Some(false),
                objective_type: Some("Earth".to_string()),
            })
        );
    }

    #[test]
    fn tolerates_champion_stat_schema_changes() {
        let stats: ChampionStats =
            serde_json::from_str(r#"{"abilityPower":42.0,"futureStatAddedByRiot":12.0}"#).unwrap();

        assert_eq!(stats.ability_power, 42.0);
        assert_eq!(stats.ability_haste, 0.0);
        assert_eq!(stats.extra["futureStatAddedByRiot"], 12.0);
    }
}
