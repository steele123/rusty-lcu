# rusty-lcu

[![Crates.io](https://img.shields.io/crates/v/rusty-lcu.svg)](https://crates.io/crates/rusty-lcu)
[![Docs.rs](https://docs.rs/rusty-lcu/badge.svg)](https://docs.rs/rusty-lcu)

A library for interacting with the LCU API in Rust. It provides a typed interface to the LCU endpoints, as well as
utilities for polling, event streams, and credential management.

The endpoint layer is generated from `schema/swagger.json`, which is vendored
from the Dysolix LCU swagger data. To regenerate from a newer schema, run
`scripts\update-swagger.ps1` on Windows or `./scripts/update-swagger.sh` on
Unix-like shells, then build normally. You can also set `LCU_SWAGGER_PATH`
before building to generate from another local schema without replacing the
vendored file.

## Installation

```toml
[dependencies]
rusty-lcu = "0.1.0"
```

Package: [crates.io/crates/rusty-lcu](https://crates.io/crates/rusty-lcu)

## Quick Start

```rust
use rusty_lcu::{generated, EndpointParams, LcuClient};

#[tokio::main]
async fn main() -> rusty_lcu::Result<()> {
    let mut client = LcuClient::new()?;
    client.connect().await?;

    let summoner = generated::get_lol_summoner_v1_current_summoner_typed(
        &client,
        EndpointParams::new(),
    )
    .await?;

    println!("{}", summoner.summoner_id);
    Ok(())
}
```

## Path, Query, And Body Params

Generated functions take `EndpointParams` so every swagger endpoint has the same
stable call shape. Endpoints with a named swagger response model also get a
`*_typed` helper that returns a generated `models::*` type. Endpoints with a
named swagger request body model also get `*_with_body` helpers, and endpoints
with both named request and response models get `*_with_body_typed`.
Required path parameters and required query parameters from the swagger are
validated before the request is sent.

```rust
let summoner = generated::get_lol_summoner_v1_summoners_by_id(
    &client,
    EndpointParams::new().path("id", 123456789),
)
.await?;

let typed_summoner = generated::get_lol_summoner_v1_summoners_by_id_typed(
    &client,
    EndpointParams::new().path("id", 123456789),
)
.await?;

let result = generated::post_lol_lobby_v2_lobby(
    &client,
    EndpointParams::new().body(serde_json::json!({ "queueId": 430 }))?,
)
.await?;

let conversation: generated::models::LolChatConversationResource = todo!();
let updated = generated::put_lol_chat_v1_conversations_by_id_with_body_typed(
    &client,
    EndpointParams::new().path("id", &conversation.id),
    &conversation,
)
.await?;
```

For raw escape-hatch calls, use `client.get`, `client.post`, `client.put`,
`client.patch`, `client.delete`, or `client.request`. Typed raw helpers are
available as `get_as`, `post_as`, `put_as`, `patch_as`, `delete_as`, and
`request_as`.

```rust
let phase: String = client
    .get_as("/lol-gameflow/v1/gameflow-phase")
    .await?;
```

## Endpoint Discovery

```rust
use rusty_lcu::generated::{
    SCHEMA_VERSION, TAGS, endpoints_for_tag, find_endpoint,
};

let endpoint = find_endpoint("GET", "/lol-summoner/v1/current-summoner")
    .expect("generated endpoint");

println!("{} {:?}", endpoint.operation_id, endpoint.response_type);
println!("generated from LCU schema {SCHEMA_VERSION}");

for endpoint in endpoints_for_tag("Plugin lol-summoner") {
    println!("{} {}", endpoint.method, endpoint.path);
}

println!("available tag groups: {}", TAGS.len());
```

You can also inspect the generated surface without a running League client:

```powershell
cargo run --example list_endpoints
```

With League running, this example connects to LCU and fetches the current
summoner through a generated typed endpoint:

```powershell
cargo run --example current_summoner
```

## Polling

```rust
use rusty_lcu::{generated, EndpointParams, PollEvent, PollOptions};

client
    .poll_endpoint(
        &generated::GET_LOL_GAMEFLOW_V1_GAMEFLOW_PHASE,
        EndpointParams::new(),
        PollOptions::default(),
        |event| match event {
            PollEvent::DistinctResponse(phase) => phase == serde_json::json!("InProgress"),
            PollEvent::Response(_) | PollEvent::Error(_) => false,
        },
    )
    .await?;
```

## Credentials

By default `connect()` tries to read credentials from the running
`LeagueClientUx` process, falls back to the League lockfile at
`C:\Riot Games\League of Legends\lockfile` on Windows, and waits for LCU to
respond. You can override the lockfile with `RUSTY_LCU_LOCKFILE`, pass a
specific lockfile path, pass lockfile content, or construct manual credentials:

```rust
use rusty_lcu::{Credentials, CredentialsSource, LcuClient};

let mut client = LcuClient::new()?;
client
    .connect_with(CredentialsSource::Manual(Credentials::new(51111, "password")))
    .await?;
```

Use `connect_with_options` to disable or customize the readiness check:

```rust
use rusty_lcu::{ConnectOptions, CredentialsSource, LcuClient};

let mut client = LcuClient::new()?;
client
    .connect_with_options(
        CredentialsSource::Process,
        ConnectOptions {
            readiness_check: None,
        },
    )
    .await?;
```

## Events

```rust
use serde::Deserialize;
use rusty_lcu::{EventFilter, EventStream};

#[derive(Debug, Deserialize)]
struct GameflowPhase(String);

let mut events = client.event_stream().await?;
events.subscribe("OnJsonApiEvent").await?;

let filter = EventFilter::new()
    .uri("/lol-gameflow/v1/gameflow-phase")
    .event_type("Update");

while let Some(phase) = events.next_matching_event_as::<GameflowPhase>(&filter).await? {
    println!("{phase:?}");
}
```

## Current Scope

This crate currently generates endpoint functions, endpoint metadata, and Rust
models for the full swagger surface. Generic functions return `serde_json::Value`;
typed helpers are generated where a request or response points at a named
swagger component schema. Exotic inline OpenAPI shapes are represented as
`serde_json::Value` inside generated models.
