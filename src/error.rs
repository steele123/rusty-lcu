use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("League Client credentials were not found")]
    CredentialsNotFound,

    #[error("invalid League Client lockfile content")]
    InvalidLockfile,

    #[error("the client is not connected")]
    NotConnected,

    #[error("missing required path parameter `{name}` for {method} {path}")]
    MissingPathParameter {
        method: &'static str,
        path: &'static str,
        name: &'static str,
    },

    #[error("missing required query parameter `{name}` for {method} {path}")]
    MissingQueryParameter {
        method: &'static str,
        path: &'static str,
        name: &'static str,
    },

    #[error("LCU did not become ready after {attempts} attempts")]
    ReadinessCheckFailed { attempts: usize },

    #[error("LCU returned {status}: {body}")]
    Lcu {
        status: reqwest::StatusCode,
        body: String,
    },

    #[cfg(feature = "live-client")]
    #[error("Live Client Data API returned {status}: {body}")]
    LiveClientData {
        status: reqwest::StatusCode,
        body: String,
    },

    #[cfg(feature = "live-client")]
    #[error("timed out waiting {timeout:?} for a League game")]
    LiveClientWaitTimedOut { timeout: std::time::Duration },

    #[cfg(feature = "live-client")]
    #[error("operation was cancelled")]
    OperationCancelled,

    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[cfg(feature = "websocket")]
    #[error("websocket failed: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("io failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("json failed: {0}")]
    Json(#[from] serde_json::Error),

    #[cfg(feature = "websocket")]
    #[error("invalid header value: {0}")]
    Header(#[from] http::header::InvalidHeaderValue),

    #[cfg(feature = "websocket")]
    #[error("http request build failed: {0}")]
    Http(#[from] http::Error),

    #[error("url parse failed: {0}")]
    Url(#[from] url::ParseError),
}
