//! Native controller transport. The renderer never opens a network connection.
//!
//! mihomo deliberately skips authentication on Unix sockets and chmods them
//! 0666, including when running as root. A fresh 0700, user-owned directory
//! protects the socket for its entire lifetime (including config reloads).

use futures_util::{SinkExt, StreamExt};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, io::Read, sync::Mutex, time::Duration};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const MAX_RESPONSE: usize = 32 * 1024 * 1024;
pub const USE_UNIX_SOCKET: bool = cfg!(all(unix, desktop));

#[derive(Clone)]
enum Endpoint {
    #[cfg_attr(all(unix, desktop), allow(dead_code))]
    // Used by Windows/Android and transport tests.
    Tcp(u16),
    #[cfg(all(unix, desktop))]
    Unix(std::sync::Arc<tempfile::TempDir>),
}

#[derive(Clone)]
pub struct Controller {
    endpoint: Endpoint,
    client: reqwest::blocking::Client,
    secret: String,
}

impl Controller {
    /// Called on the blocking pool, before starting either child or service.
    pub fn new(port: u16, secret: &str) -> Result<Self, String> {
        #[cfg(all(unix, desktop))]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = port;
            // Keep well below sockaddr_un's pathname limit, also on macOS.
            let dir = tempfile::Builder::new()
                .prefix("mfork-ctl-")
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir_in("/tmp")
                .map_err(|e| format!("create private controller directory: {e}"))?;
            return Self::from_endpoint(Endpoint::Unix(std::sync::Arc::new(dir)), secret);
        }
        #[cfg(not(all(unix, desktop)))]
        Self::from_endpoint(Endpoint::Tcp(port), secret)
    }

    fn from_endpoint(endpoint: Endpoint, secret: &str) -> Result<Self, String> {
        let builder = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(60));
        #[cfg(all(unix, desktop))]
        let builder = match &endpoint {
            Endpoint::Unix(dir) => builder.unix_socket(dir.path().join("api.sock")),
            Endpoint::Tcp(_) => builder,
        };
        Ok(Self {
            endpoint,
            client: builder.build().map_err(|e| e.to_string())?,
            secret: secret.into(),
        })
    }

    pub fn transport(&self) -> &'static str {
        match self.endpoint {
            Endpoint::Tcp(_) => "tcp",
            #[cfg(all(unix, desktop))]
            Endpoint::Unix(_) => "unix",
        }
    }

    pub fn configure(&self, doc: &mut serde_yaml::Mapping) {
        // Overrides/subscriptions must not add a second, unprotected controller.
        for key in [
            "external-controller",
            "external-controller-tls",
            "external-controller-unix",
            "external-controller-pipe",
        ] {
            doc.remove(serde_yaml::Value::String(key.into()));
        }
        match &self.endpoint {
            Endpoint::Tcp(port) => {
                doc.insert(
                    "external-controller".into(),
                    format!("127.0.0.1:{port}").into(),
                );
            }
            #[cfg(all(unix, desktop))]
            Endpoint::Unix(dir) => {
                doc.insert(
                    "external-controller-unix".into(),
                    dir.path()
                        .join("api.sock")
                        .to_string_lossy()
                        .as_ref()
                        .into(),
                );
            }
        }
    }

    fn url(&self, path: &str) -> Result<String, String> {
        validate_path(path)?;
        let port = match self.endpoint {
            Endpoint::Tcp(port) => port,
            #[cfg(all(unix, desktop))]
            Endpoint::Unix(_) => 1, // HTTP authority only; transport is exclusively AF_UNIX.
        };
        Ok(format!("http://127.0.0.1:{port}{path}"))
    }

    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let method = match method {
            "GET" => Method::GET,
            "PUT" => Method::PUT,
            "PATCH" => Method::PATCH,
            "POST" => Method::POST,
            "DELETE" => Method::DELETE,
            _ => return Err("unsupported controller method".into()),
        };
        let mut req = self
            .client
            .request(method, self.url(path)?)
            .bearer_auth(&self.secret)
            .timeout(timeout);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req
            .send()
            .map_err(|e| format!("{path} ({}): {e}", self.transport()))?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take((MAX_RESPONSE + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{path}: read response: {e}"))?;
        if bytes.len() > MAX_RESPONSE {
            return Err(format!("{path}: controller response too large"));
        }
        if !status.is_success() {
            let text = String::from_utf8_lossy(&bytes);
            let detail = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| text.into_owned());
            return Err(format!(
                "{path}: HTTP {} — {}",
                status.as_u16(),
                detail.chars().take(500).collect::<String>()
            ));
        }
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&bytes).map_err(|e| format!("{path}: invalid controller JSON: {e}"))
    }

    pub fn ready(&self) -> Result<(), String> {
        let version = self.request("GET", "/version", None, Duration::from_secs(1))?;
        let config = self.request("GET", "/configs", None, Duration::from_secs(1))?;
        if !version["version"].is_string()
            || !matches!(config["mode"].as_str(), Some("rule" | "global" | "direct"))
        {
            return Err("controller returned incomplete startup state".into());
        }
        Ok(())
    }

    async fn stream_once(
        &self,
        kind: StreamKind,
        send: &impl Fn(StreamEvent) -> Result<(), String>,
    ) -> Result<(), String> {
        let path = kind.path();
        let connect = async {
            let socket: Box<dyn NativeSocket> = match &self.endpoint {
                Endpoint::Tcp(port) => Box::new(
                    tokio::net::TcpStream::connect(("127.0.0.1", *port))
                        .await
                        .map_err(|e| e.to_string())?,
                ),
                #[cfg(all(unix, desktop))]
                Endpoint::Unix(dir) => Box::new(
                    tokio::net::UnixStream::connect(dir.path().join("api.sock"))
                        .await
                        .map_err(|e| e.to_string())?,
                ),
            };
            let mut request = self
                .url(path)?
                .replacen("http:", "ws:", 1)
                .into_client_request()
                .map_err(|e| e.to_string())?;
            request.headers_mut().insert(
                "Authorization",
                format!("Bearer {}", self.secret)
                    .parse()
                    .map_err(|e| format!("invalid controller auth: {e}"))?,
            );
            tokio_tungstenite::client_async(request, socket)
                .await
                .map_err(|e| e.to_string())
        };
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), connect)
            .await
            .map_err(|_| format!("{path}: controller handshake timed out"))??;
        send(StreamEvent::Connected)?;
        while let Some(message) = socket.next().await {
            match message.map_err(|e| format!("{path}: {e}"))? {
                Message::Text(text) => {
                    if text.len() > MAX_RESPONSE {
                        return Err(format!("{path}: event too large"));
                    }
                    let data = serde_json::from_str(&text)
                        .map_err(|e| format!("{path}: invalid event: {e}"))?;
                    send(StreamEvent::Data { data })?;
                }
                Message::Ping(data) => socket
                    .send(Message::Pong(data))
                    .await
                    .map_err(|e| e.to_string())?,
                Message::Close(_) => break,
                _ => {}
            }
        }
        Err(format!("{path}: controller stream closed"))
    }

    async fn stream(&self, kind: StreamKind, send: impl Fn(StreamEvent) -> Result<(), String>) {
        let mut delay = 1;
        loop {
            let started = std::time::Instant::now();
            let error = self
                .stream_once(kind, &send)
                .await
                .err()
                .unwrap_or_else(|| "controller stream ended".into());
            if send(StreamEvent::Error {
                message: format!("{} ({}): {error}", kind.path(), self.transport()),
            })
            .is_err()
            {
                return;
            }
            if started.elapsed() > Duration::from_secs(15) {
                delay = 1;
            }
            tokio::time::sleep(Duration::from_secs(delay)).await;
            delay = (delay * 2).min(15);
        }
    }
}

trait NativeSocket: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> NativeSocket for T {}

fn validate_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.contains(['\\', '#'])
        || path.chars().any(char::is_control)
    {
        return Err("controller path must be a local API path".into());
    }
    let url = reqwest::Url::parse(&format!("http://localhost{path}")).map_err(|e| e.to_string())?;
    let root = url.path().split('/').nth(1).unwrap_or_default();
    if url.path() != "/upgrade/geo"
        && !matches!(
            root,
            "version"
                | "configs"
                | "proxies"
                | "group"
                | "rules"
                | "connections"
                | "providers"
                | "cache"
                | "dns"
                | "logs"
                | "traffic"
                | "memory"
        )
    {
        return Err("unsupported controller path".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    Logs,
    Traffic,
    Memory,
    Connections,
}
impl StreamKind {
    fn path(self) -> &'static str {
        match self {
            Self::Logs => "/logs?level=debug",
            Self::Traffic => "/traffic",
            Self::Memory => "/memory",
            Self::Connections => "/connections",
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum StreamEvent {
    Connected,
    Data { data: Value },
    Error { message: String },
}

#[derive(Default)]
pub struct Streams(Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>);
impl Streams {
    pub fn stop_all(&self) {
        self.0.lock().unwrap().clear();
    }
}

#[tauri::command]
pub async fn core_request(
    app: AppHandle,
    session_id: String,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, String> {
    let api = app
        .state::<crate::core::CoreState>()
        .api
        .lock()
        .unwrap()
        .clone()
        .filter(|api| api.id == session_id)
        .ok_or("core session has stopped or changed")?;
    let timeout = if path == "/upgrade/geo" { 180 } else { 60 };
    tauri::async_runtime::spawn_blocking(move || {
        api.controller
            .request(&method, &path, body, Duration::from_secs(timeout))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn core_subscribe(
    app: AppHandle,
    session_id: String,
    subscription_id: String,
    kind: StreamKind,
    on_event: Channel<StreamEvent>,
) -> Result<(), String> {
    let state = app.state::<crate::core::CoreState>();
    let api_guard = state.api.lock().unwrap();
    let api = api_guard
        .as_ref()
        .filter(|api| api.id == session_id)
        .ok_or("core session has stopped or changed")?;
    let streams = app.state::<Streams>();
    let mut subscriptions = streams.0.lock().unwrap();
    subscriptions.retain(|_, tx| !tx.is_closed());
    if subscriptions.len() >= 16 {
        return Err("too many controller subscriptions".into());
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    subscriptions.insert(subscription_id, tx);
    let controller = api.controller.clone();
    tauri::async_runtime::spawn(async move {
        tokio::select! {
            _ = rx => {},
            _ = controller.stream(kind, |event| on_event.send(event).map_err(|e| e.to_string())) => {},
        }
    });
    Ok(())
}

#[tauri::command]
pub fn core_unsubscribe(streams: State<'_, Streams>, subscription_id: String) {
    streams.0.lock().unwrap().remove(&subscription_id);
}

#[cfg(test)]
#[path = "controller_tests.rs"]
mod tests;
