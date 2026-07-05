//! Rust helpers for the League Client Update (LCU) API.
//!
//! Endpoint wrappers are generated at build time from `schema/swagger.json`.

mod client;
mod credentials;
mod error;
mod events;

pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/lcu_endpoints.rs"));
}

pub use client::{
    ConnectOptions, EndpointParams, LcuClient, PollEvent, PollOptions, ReadinessCheck,
    RequestOptions,
};
pub use credentials::{Credentials, CredentialsSource};
pub use error::{Error, Result};
pub use events::{EventFilter, EventStream, LcuEvent};

#[cfg(test)]
mod tests {
    use super::generated::{
        ENDPOINTS, GET_LOL_SUMMONER_V1_CURRENT_SUMMONER, PUT_LOL_CHAT_V1_CONVERSATIONS_BY_ID,
        SCHEMA_TITLE, SCHEMA_UPSTREAM_URL, SCHEMA_VERSION, TAGS, endpoints_for_tag, find_endpoint,
        find_endpoint_by_operation_id, models,
    };

    #[test]
    fn swagger_generates_lcu_endpoints() {
        assert_eq!(SCHEMA_TITLE, "LCU SCHEMA");
        assert!(!SCHEMA_VERSION.is_empty());
        assert!(SCHEMA_UPSTREAM_URL.contains("dysolix/hasagi-types"));
        assert!(ENDPOINTS.len() > 1_000);
        assert!(ENDPOINTS.iter().any(|endpoint| {
            endpoint.method == "GET" && endpoint.path == "/lol-summoner/v1/current-summoner"
        }));
        assert_eq!(
            GET_LOL_SUMMONER_V1_CURRENT_SUMMONER.response_type,
            Some("LolSummonerSummoner")
        );
    }

    #[test]
    fn swagger_generates_lcu_models() {
        let summoner: models::LolSummonerSummoner = serde_json::from_value(serde_json::json!({
            "accountId": 1,
            "displayName": "steel",
            "gameName": "steel",
            "internalName": "steel",
            "nameChangeFlag": false,
            "percentCompleteForNextLevel": 50,
            "privacy": "PUBLIC",
            "profileIconId": 29,
            "puuid": "puuid",
            "rerollPoints": {
                "currentPoints": 1,
                "maxRolls": 2,
                "numberOfRolls": 0,
                "pointsCostToRoll": 250,
                "pointsToReroll": 249
            },
            "summonerId": 2,
            "summonerLevel": 30,
            "tagLine": "NA1",
            "unnamed": false,
            "xpSinceLastLevel": 1,
            "xpUntilNextLevel": 2
        }))
        .unwrap();

        assert_eq!(summoner.summoner_id, 2);
        assert_eq!(summoner.reroll_points.max_rolls, 2);
    }

    #[test]
    fn swagger_generates_endpoint_discovery_helpers() {
        let endpoint = find_endpoint("GET", "/lol-summoner/v1/current-summoner").unwrap();
        assert_eq!(endpoint.operation_id, "GetLolSummonerV1CurrentSummoner");

        let templated = find_endpoint("GET", "/lol-summoner/v1/summoners/123").unwrap();
        assert_eq!(templated.path, "/lol-summoner/v1/summoners/{id}");

        assert_eq!(
            find_endpoint_by_operation_id("GetLolSummonerV1CurrentSummoner"),
            Some(&GET_LOL_SUMMONER_V1_CURRENT_SUMMONER)
        );

        assert!(TAGS.contains(&"Plugin lol-summoner"));
        assert!(
            endpoints_for_tag("Plugin lol-summoner")
                .any(|endpoint| endpoint.path == "/lol-summoner/v1/current-summoner")
        );
    }

    #[test]
    fn swagger_generates_typed_request_metadata() {
        assert_eq!(
            PUT_LOL_CHAT_V1_CONVERSATIONS_BY_ID.request_type,
            Some("LolChatConversationResource")
        );
        assert_eq!(
            PUT_LOL_CHAT_V1_CONVERSATIONS_BY_ID.response_type,
            Some("LolChatConversationResource")
        );

        let _helper = super::generated::put_lol_chat_v1_conversations_by_id_with_body;
        let _typed_helper = super::generated::put_lol_chat_v1_conversations_by_id_with_body_typed;
    }
}
