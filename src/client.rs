use std::time::Duration;

use reqwest::{Method, Url};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::time::sleep;

use crate::{
    Credentials, CredentialsSource, Error, Result, events::EventStream, generated::Endpoint,
};

#[derive(Debug, Clone)]
pub struct LcuClient {
    http: reqwest::Client,
    credentials: Option<Credentials>,
    default_request_options: RequestOptions,
}

#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub max_retries: usize,
    pub retry_delay: Duration,
    pub retry_on_statuses: Vec<reqwest::StatusCode>,
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            max_retries: 0,
            retry_delay: Duration::from_millis(500),
            retry_on_statuses: vec![
                reqwest::StatusCode::TOO_MANY_REQUESTS,
                reqwest::StatusCode::BAD_GATEWAY,
                reqwest::StatusCode::SERVICE_UNAVAILABLE,
                reqwest::StatusCode::GATEWAY_TIMEOUT,
            ],
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EndpointParams {
    path: Vec<(String, String)>,
    query: Vec<(String, String)>,
    body: Option<Value>,
    request_options: Option<RequestOptions>,
}

impl EndpointParams {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn path(mut self, name: impl Into<String>, value: impl ToString) -> Self {
        self.path.push((name.into(), value.to_string()));
        self
    }

    pub fn query(mut self, name: impl Into<String>, value: impl ToString) -> Self {
        self.query.push((name.into(), value.to_string()));
        self
    }

    pub fn body(mut self, value: impl Serialize) -> Result<Self> {
        self.body = Some(serde_json::to_value(value)?);
        Ok(self)
    }

    pub fn request_options(mut self, options: RequestOptions) -> Self {
        self.request_options = Some(options);
        self
    }
}

#[derive(Debug, Clone)]
pub struct PollOptions {
    pub interval: Duration,
    pub max_attempts: Option<usize>,
    pub stop_on_error: bool,
}

impl Default for PollOptions {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            max_attempts: None,
            stop_on_error: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PollEvent<T> {
    Response(T),
    DistinctResponse(T),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub readiness_check: Option<ReadinessCheck>,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self {
            readiness_check: Some(ReadinessCheck::default()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReadinessCheck {
    pub path: String,
    pub max_attempts: usize,
    pub delay: Duration,
}

impl Default for ReadinessCheck {
    fn default() -> Self {
        Self {
            path: "/lol-gameflow/v1/gameflow-phase".to_string(),
            max_attempts: 10,
            delay: Duration::from_secs(1),
        }
    }
}

impl LcuClient {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()?;

        Ok(Self {
            http,
            credentials: None,
            default_request_options: RequestOptions::default(),
        })
    }

    pub fn with_credentials(credentials: Credentials) -> Result<Self> {
        let mut client = Self::new()?;
        client.credentials = Some(credentials);
        Ok(client)
    }

    pub fn set_default_request_options(&mut self, options: RequestOptions) {
        self.default_request_options = options;
    }

    pub async fn connect(&mut self) -> Result<()> {
        self.connect_with_options(CredentialsSource::Auto, ConnectOptions::default())
            .await
    }

    pub async fn connect_with(&mut self, source: CredentialsSource) -> Result<()> {
        self.credentials = Some(Credentials::discover(source).await?);
        Ok(())
    }

    pub async fn connect_with_options(
        &mut self,
        source: CredentialsSource,
        options: ConnectOptions,
    ) -> Result<()> {
        self.credentials = Some(Credentials::discover(source).await?);

        if let Some(readiness_check) = options.readiness_check {
            self.wait_until_ready(readiness_check).await?;
        }

        Ok(())
    }

    pub fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    pub async fn event_stream(&self) -> Result<EventStream> {
        let credentials = self.credentials.as_ref().ok_or(Error::NotConnected)?;
        EventStream::connect(credentials).await
    }

    pub async fn wait_until_ready(&self, readiness_check: ReadinessCheck) -> Result<()> {
        let attempts = readiness_check.max_attempts.max(1);

        for attempt in 0..attempts {
            if self.get(&readiness_check.path).await.is_ok() {
                return Ok(());
            }

            if attempt + 1 < attempts {
                sleep(readiness_check.delay).await;
            }
        }

        Err(Error::ReadinessCheckFailed { attempts })
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        self.request(Method::GET, path, EndpointParams::new()).await
    }

    pub async fn get_as<T>(&self, path: &str) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.request_as(Method::GET, path, EndpointParams::new())
            .await
    }

    pub async fn post(&self, path: &str, params: EndpointParams) -> Result<Value> {
        self.request(Method::POST, path, params).await
    }

    pub async fn post_as<T>(&self, path: &str, params: EndpointParams) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.request_as(Method::POST, path, params).await
    }

    pub async fn put(&self, path: &str, params: EndpointParams) -> Result<Value> {
        self.request(Method::PUT, path, params).await
    }

    pub async fn put_as<T>(&self, path: &str, params: EndpointParams) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.request_as(Method::PUT, path, params).await
    }

    pub async fn patch(&self, path: &str, params: EndpointParams) -> Result<Value> {
        self.request(Method::PATCH, path, params).await
    }

    pub async fn patch_as<T>(&self, path: &str, params: EndpointParams) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.request_as(Method::PATCH, path, params).await
    }

    pub async fn delete(&self, path: &str, params: EndpointParams) -> Result<Value> {
        self.request(Method::DELETE, path, params).await
    }

    pub async fn delete_as<T>(&self, path: &str, params: EndpointParams) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.request_as(Method::DELETE, path, params).await
    }

    pub async fn request_endpoint(
        &self,
        endpoint: &'static Endpoint,
        params: EndpointParams,
    ) -> Result<Value> {
        validate_endpoint_params(endpoint, &params)?;
        let method = Method::from_bytes(endpoint.method.as_bytes()).expect("generated method");
        self.request(method, endpoint.path, params).await
    }

    pub async fn request_endpoint_as<T>(
        &self,
        endpoint: &'static Endpoint,
        params: EndpointParams,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        parse_value(self.request_endpoint(endpoint, params).await?)
    }

    pub async fn poll_endpoint<F>(
        &self,
        endpoint: &'static Endpoint,
        params: EndpointParams,
        options: PollOptions,
        mut callback: F,
    ) -> Result<()>
    where
        F: FnMut(PollEvent<Value>) -> bool,
    {
        self.poll_endpoint_as(endpoint, params, options, |event| callback(event))
            .await
    }

    pub async fn poll_endpoint_as<T, F>(
        &self,
        endpoint: &'static Endpoint,
        params: EndpointParams,
        options: PollOptions,
        mut callback: F,
    ) -> Result<()>
    where
        T: DeserializeOwned + Clone + PartialEq,
        F: FnMut(PollEvent<T>) -> bool,
    {
        let mut attempts = 0;
        let mut previous = None;

        loop {
            if options
                .max_attempts
                .is_some_and(|max_attempts| attempts >= max_attempts)
            {
                return Ok(());
            }
            attempts += 1;

            match self
                .request_endpoint_as::<T>(endpoint, params.clone())
                .await
            {
                Ok(value) => {
                    let is_distinct = previous.as_ref() != Some(&value);
                    previous = Some(value.clone());

                    if callback(PollEvent::Response(value.clone())) {
                        return Ok(());
                    }
                    if is_distinct && callback(PollEvent::DistinctResponse(value)) {
                        return Ok(());
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    if callback(PollEvent::Error(message)) || options.stop_on_error {
                        return Err(error);
                    }
                }
            }

            sleep(options.interval).await;
        }
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        params: EndpointParams,
    ) -> Result<Value> {
        let credentials = self.credentials.as_ref().ok_or(Error::NotConnected)?;
        let options = params
            .request_options
            .clone()
            .unwrap_or_else(|| self.default_request_options.clone());

        let url = build_url(credentials, path, &params.path, &params.query)?;
        let mut last_error = None;

        for attempt in 0..=options.max_retries {
            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .basic_auth("riot", Some(&credentials.password));

            if let Some(body) = params.body.clone() {
                request = request.json(&body);
            }

            match request.send().await {
                Ok(response) if response.status().is_success() => {
                    return parse_success_response(response).await;
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    if attempt < options.max_retries && options.retry_on_statuses.contains(&status)
                    {
                        sleep(options.retry_delay).await;
                        continue;
                    }
                    return Err(Error::Lcu { status, body });
                }
                Err(error) => {
                    last_error = Some(error);
                    if attempt < options.max_retries {
                        sleep(options.retry_delay).await;
                        continue;
                    }
                }
            }
        }

        Err(Error::Request(last_error.expect("request attempted")))
    }

    pub async fn request_as<T>(
        &self,
        method: Method,
        path: &str,
        params: EndpointParams,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        parse_value(self.request(method, path, params).await?)
    }
}

fn build_url(
    credentials: &Credentials,
    path: &str,
    path_params: &[(String, String)],
    query_params: &[(String, String)],
) -> Result<Url> {
    let mut rendered = path.to_string();
    for (name, value) in path_params {
        rendered = rendered.replace(
            &format!("{{{name}}}"),
            &urlencoding::encode(value).into_owned(),
        );
    }

    let mut url = Url::parse(&format!("{}{}", credentials.base_url(), rendered))?;

    for (name, value) in query_params {
        url.query_pairs_mut().append_pair(name, value);
    }

    Ok(url)
}

fn validate_endpoint_params(endpoint: &'static Endpoint, params: &EndpointParams) -> Result<()> {
    for required_path_param in endpoint.path_params {
        if !params
            .path
            .iter()
            .any(|(name, _)| name == required_path_param)
        {
            return Err(Error::MissingPathParameter {
                method: endpoint.method,
                path: endpoint.path,
                name: required_path_param,
            });
        }
    }

    for required_query_param in endpoint.required_query_params {
        if !params
            .query
            .iter()
            .any(|(name, _)| name == required_query_param)
        {
            return Err(Error::MissingQueryParameter {
                method: endpoint.method,
                path: endpoint.path,
                name: required_query_param,
            });
        }
    }

    Ok(())
}

async fn parse_success_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let bytes = response.bytes().await?;

    if bytes.is_empty() || status == reqwest::StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }

    Ok(serde_json::from_slice(&bytes)?)
}

fn parse_value<T>(value: Value) -> Result<T>
where
    T: DeserializeOwned,
{
    Ok(serde_json::from_value(value)?)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use serde::Deserialize;
    use serde_json::json;

    use crate::{
        Credentials, EndpointParams, Error, LcuClient,
        generated::{GET_LOL_CATALOG_V1_ITEM_DETAILS, GET_LOL_SUMMONER_V1_SUMMONERS_BY_ID},
    };

    use super::{parse_value, validate_endpoint_params};

    #[test]
    fn validates_generated_path_params() {
        let error =
            validate_endpoint_params(&GET_LOL_SUMMONER_V1_SUMMONERS_BY_ID, &EndpointParams::new())
                .unwrap_err();

        assert!(matches!(
            error,
            Error::MissingPathParameter {
                method: "GET",
                path: "/lol-summoner/v1/summoners/{id}",
                name: "id"
            }
        ));

        validate_endpoint_params(
            &GET_LOL_SUMMONER_V1_SUMMONERS_BY_ID,
            &EndpointParams::new().path("id", 123),
        )
        .unwrap();
    }

    #[test]
    fn validates_generated_required_query_params() {
        let error =
            validate_endpoint_params(&GET_LOL_CATALOG_V1_ITEM_DETAILS, &EndpointParams::new())
                .unwrap_err();

        assert!(matches!(
            error,
            Error::MissingQueryParameter {
                method: "GET",
                path: "/lol-catalog/v1/item-details",
                name: "inventoryType"
            }
        ));

        validate_endpoint_params(
            &GET_LOL_CATALOG_V1_ITEM_DETAILS,
            &EndpointParams::new()
                .query("inventoryType", "CHAMPION")
                .query("itemId", 1),
        )
        .unwrap();
    }

    #[test]
    fn parses_typed_values_for_raw_helpers() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Phase {
            phase: String,
        }

        let phase: Phase = parse_value(json!({ "phase": "Lobby" })).unwrap();
        assert_eq!(
            phase,
            Phase {
                phase: "Lobby".to_string()
            }
        );
    }

    #[test]
    fn raw_typed_requests_send_auth_query_and_body() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct EchoResponse {
            ok: bool,
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let bytes_read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..bytes_read]);

            assert!(request.starts_with("POST /test?answer=42 HTTP/1.1"));
            assert!(request.contains("authorization: Basic cmlvdDpzZWNyZXQ="));
            assert!(request.contains("content-type: application/json"));
            assert!(request.contains(r#"{"hello":"world"}"#));

            let body = r#"{"ok":true}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let response = runtime
            .block_on(async {
                let client = LcuClient::with_credentials(Credentials {
                    port,
                    password: "secret".to_string(),
                    protocol: "http".to_string(),
                })
                .unwrap();

                client
                    .post_as::<EchoResponse>(
                        "/test",
                        EndpointParams::new()
                            .query("answer", 42)
                            .body(json!({ "hello": "world" }))
                            .unwrap(),
                    )
                    .await
            })
            .unwrap();

        server.join().unwrap();
        assert_eq!(response, EchoResponse { ok: true });
    }
}
