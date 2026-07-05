use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use http::{HeaderValue, Request, header::AUTHORIZATION};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config, tungstenite::Message,
};

use crate::{Credentials, Result};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LcuEvent {
    pub data: Value,
    pub event_type: String,
    pub uri: String,
}

impl LcuEvent {
    pub fn data_as<T>(&self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        Ok(serde_json::from_value(self.data.clone())?)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventFilter {
    pub uri: Option<String>,
    pub event_types: Vec<String>,
    pub name: Option<String>,
}

impl EventFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
        self
    }

    pub fn event_type(mut self, event_type: impl Into<String>) -> Self {
        self.event_types.push(event_type.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn matches(&self, event: &LcuEvent) -> bool {
        if self.uri.as_deref().is_some_and(|uri| uri != event.uri) {
            return false;
        }

        if !self.event_types.is_empty()
            && !self
                .event_types
                .iter()
                .any(|event_type| event_type == &event.event_type)
        {
            return false;
        }

        if let Some(name) = &self.name {
            let event_name = event
                .data
                .get("name")
                .and_then(Value::as_str)
                .or_else(|| event.data.get("eventName").and_then(Value::as_str));

            if event_name != Some(name.as_str()) {
                return false;
            }
        }

        true
    }
}

pub struct EventStream {
    write: SplitSink<Socket, Message>,
    read: SplitStream<Socket>,
}

impl EventStream {
    pub async fn connect(credentials: &Credentials) -> Result<Self> {
        let authorization = STANDARD.encode(format!("riot:{}", credentials.password));
        let request = Request::builder()
            .uri(credentials.websocket_url())
            .header(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Basic {authorization}"))?,
            )
            .body(())?;

        let connector = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .map_err(|error| std::io::Error::other(error.to_string()))?;

        let (socket, _) = connect_async_tls_with_config(
            request,
            None,
            false,
            Some(Connector::NativeTls(connector)),
        )
        .await?;

        let (write, read) = socket.split();
        Ok(Self { write, read })
    }

    pub async fn subscribe_all(&mut self) -> Result<()> {
        self.subscribe("OnJsonApiEvent").await
    }

    pub async fn subscribe(&mut self, event_name: &str) -> Result<()> {
        self.send_subscription_message(5, event_name).await
    }

    pub async fn unsubscribe(&mut self, event_name: &str) -> Result<()> {
        self.send_subscription_message(6, event_name).await
    }

    pub async fn next_event(&mut self) -> Result<Option<LcuEvent>> {
        while let Some(message) = self.read.next().await {
            let message = message?;
            if !message.is_text() {
                continue;
            }

            let payload: Value = serde_json::from_str(message.to_text()?)?;
            let Some(event) = payload.get(2) else {
                continue;
            };

            return Ok(Some(serde_json::from_value(event.clone())?));
        }

        Ok(None)
    }

    pub async fn next_matching_event(&mut self, filter: &EventFilter) -> Result<Option<LcuEvent>> {
        while let Some(event) = self.next_event().await? {
            if filter.matches(&event) {
                return Ok(Some(event));
            }
        }

        Ok(None)
    }

    pub async fn next_matching_event_as<T>(&mut self, filter: &EventFilter) -> Result<Option<T>>
    where
        T: DeserializeOwned,
    {
        Ok(self
            .next_matching_event(filter)
            .await?
            .map(|event| event.data_as())
            .transpose()?)
    }

    async fn send_subscription_message(&mut self, opcode: u8, event_name: &str) -> Result<()> {
        self.write
            .send(Message::Text(
                serde_json::json!([opcode, event_name]).to_string().into(),
            ))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[test]
    fn event_filter_matches_uri_type_and_name() {
        let event = LcuEvent {
            data: json!({ "name": "OnJsonApiEvent" }),
            event_type: "Update".to_string(),
            uri: "/lol-gameflow/v1/gameflow-phase".to_string(),
        };

        let filter = EventFilter::new()
            .uri("/lol-gameflow/v1/gameflow-phase")
            .event_type("Update")
            .name("OnJsonApiEvent");

        assert!(filter.matches(&event));
        assert!(!EventFilter::new().event_type("Delete").matches(&event));
        assert!(!EventFilter::new().uri("/other").matches(&event));
    }

    #[test]
    fn event_data_deserializes_to_typed_payload() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct PhasePayload {
            phase: String,
        }

        let event = LcuEvent {
            data: json!({ "phase": "ReadyCheck" }),
            event_type: "Update".to_string(),
            uri: "/lol-gameflow/v1/gameflow-phase".to_string(),
        };

        let payload: PhasePayload = event.data_as().unwrap();
        assert_eq!(
            payload,
            PhasePayload {
                phase: "ReadyCheck".to_string()
            }
        );
    }
}
