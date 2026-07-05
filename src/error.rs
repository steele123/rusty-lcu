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

    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("websocket failed: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("io failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("json failed: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid header value: {0}")]
    Header(#[from] http::header::InvalidHeaderValue),

    #[error("http request build failed: {0}")]
    Http(#[from] http::Error),

    #[error("url parse failed: {0}")]
    Url(#[from] url::ParseError),
}
