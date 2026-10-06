//! Verified local Standard API shapes. No cloud/QR requests are implemented.
//! Reference: https://developer.lovense.com/docs/standard-solutions/standard-api

use std::{collections::BTreeMap, time::Duration};

use race2love_core::{config::LovenseConfig, devices::DeviceError, unit};
use serde::{Deserialize, Serialize};

pub const LEASE: Duration = Duration::from_secs(2);
pub const RENEWAL: Duration = Duration::from_millis(500);
const MAX_RESPONSE: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toy {
    pub id: String,
    pub name: String,
    pub nickname: String,
    pub connected: bool,
    pub battery: Option<u8>,
    /// Older Remote versions may omit capability lists. Selection stays explicit.
    pub vibration: Option<bool>,
}

impl Toy {
    pub fn label(&self) -> String {
        if self.nickname.is_empty() {
            format!("{} ({})", self.name, self.id)
        } else {
            format!("{} · {} ({})", self.nickname, self.name, self.id)
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    code: u16,
    #[serde(rename = "type")]
    kind: String,
    data: Option<ToyData>,
}

#[derive(Deserialize)]
struct ToyData {
    toys: ToyMap,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ToyMap {
    Encoded(String),
    Object(BTreeMap<String, RawToy>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Number {
    Integer(u8),
    Text(String),
}

impl Number {
    fn value(&self) -> Result<u8, DeviceError> {
        match self {
            Self::Integer(number) => Ok(*number),
            Self::Text(text) => text.parse().map_err(|_| malformed("invalid numeric field")),
        }
    }
}

#[derive(Deserialize)]
struct RawToy {
    id: String,
    name: String,
    status: Number,
    battery: Option<Number>,
    #[serde(default, rename = "nickName")]
    nickname: String,
    #[serde(rename = "shortFunctionNames")]
    short_functions: Option<Vec<String>>,
    #[serde(rename = "fullFunctionNames")]
    full_functions: Option<Vec<String>>,
}

fn malformed(detail: &str) -> DeviceError {
    DeviceError::Communication(format!("Remote returned invalid data: {detail}"))
}

fn envelope(bytes: &[u8]) -> Result<Envelope, DeviceError> {
    let response = decode_envelope(bytes)?;
    if response.code != 200 || !response.kind.eq_ignore_ascii_case("ok") {
        return Err(DeviceError::Communication(format!(
            "Remote rejected the command (code {})",
            response.code
        )));
    }
    Ok(response)
}

fn decode_envelope(bytes: &[u8]) -> Result<Envelope, DeviceError> {
    serde_json::from_slice(bytes).map_err(|error| {
        tracing::debug!(%error, "Malformed Lovense response");
        malformed("expected a Standard API response")
    })
}

pub fn parse_toys(bytes: &[u8]) -> Result<Vec<Toy>, DeviceError> {
    let data = envelope(bytes)?
        .data
        .ok_or_else(|| malformed("missing toys"))?;
    let raw = match data.toys {
        ToyMap::Encoded(text) => serde_json::from_str::<BTreeMap<String, RawToy>>(&text)
            .map_err(|_| malformed("invalid encoded toys object"))?,
        ToyMap::Object(map) => map,
    };
    if raw.len() > 128 {
        return Err(malformed("too many toys"));
    }
    raw.into_iter()
        .map(|(key, toy)| {
            if key != toy.id
                || toy.id.is_empty()
                || toy.id.len() > 128
                || !toy
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || toy.name.is_empty()
                || toy.name.len() > 128
                || toy.nickname.len() > 128
            {
                return Err(malformed("invalid toy identity"));
            }
            let status = toy.status.value()?;
            if status > 1 {
                return Err(malformed("unknown connection status"));
            }
            let battery = toy.battery.as_ref().map(Number::value).transpose()?;
            if battery.is_some_and(|value| value > 100) {
                return Err(malformed("invalid battery level"));
            }
            let vibration = if toy.short_functions.is_some() || toy.full_functions.is_some() {
                Some(
                    toy.short_functions
                        .as_ref()
                        .is_some_and(|names| names.iter().any(|n| n == "v"))
                        || toy.full_functions.as_ref().is_some_and(|names| {
                            names.iter().any(|n| n.eq_ignore_ascii_case("vibrate"))
                        }),
                )
            } else {
                None
            };
            Ok(Toy {
                id: toy.id,
                name: toy.name,
                nickname: toy.nickname,
                connected: status == 1,
                battery,
                vibration,
            })
        })
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Function<'a> {
    command: &'static str,
    action: String,
    toy: &'a str,
    api_ver: u8,
    time_sec: u8,
    stop_previous: u8,
}

pub fn vibration_step(intensity: f32) -> u8 {
    (unit(intensity) * 20.0).floor() as u8
}

#[derive(Clone)]
pub struct RemoteClient {
    client: reqwest::Client,
    endpoint: reqwest::Url,
}

impl RemoteClient {
    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }
    pub fn new(config: &LovenseConfig) -> Result<Self, DeviceError> {
        let host = config.host.trim();
        let port = config
            .port
            .filter(|port| *port != 0)
            .ok_or_else(|| malformed("set the port shown by Remote"))?;
        if host.is_empty()
            || host.len() > 253
            || host.contains(['/', '@', '?', '#'])
            || host.chars().any(char::is_whitespace)
        {
            return Err(malformed("enter a host or IP, without a URL/path"));
        }
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.into()
        };
        let endpoint = reqwest::Url::parse(&format!(
            "{}://{host}:{port}/command",
            config.protocol.scheme()
        ))
        .map_err(|_| malformed("invalid Remote address"))?;
        if endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            return Err(malformed("invalid Remote host"));
        }
        if !(100..=5_000).contains(&config.request_timeout_ms) {
            return Err(malformed("API timeout must be 100..=5000 ms"));
        }
        // reqwest's no-provider feature deliberately does not auto-install one.
        // Concurrent clients may race; an already installed provider is valid.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_millis(config.request_timeout_ms))
            .connect_timeout(Duration::from_millis(config.request_timeout_ms))
            .pool_max_idle_per_host(1)
            .tcp_nodelay(true)
            .user_agent("Race2Love/0.1")
            .build()
            .map_err(network_error)?;
        Ok(Self { client, endpoint })
    }

    async fn request(&self, body: &impl Serialize) -> Result<Vec<u8>, DeviceError> {
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .header("X-platform", "Race2Love")
            .json(body)
            .send()
            .await
            .map_err(network_error)?;
        if !response.status().is_success() {
            return Err(DeviceError::Communication(format!(
                "Remote HTTP status {}",
                response.status()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE as u64)
        {
            return Err(malformed("response exceeds 64 KiB"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err(malformed("response exceeds 64 KiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    pub async fn get_toys(&self) -> Result<Vec<Toy>, DeviceError> {
        #[derive(Serialize)]
        struct GetToys {
            command: &'static str,
        }
        parse_toys(&self.request(&GetToys { command: "GetToys" }).await?)
    }

    pub async fn vibrate(&self, toy: &str, intensity: f32) -> Result<(), DeviceError> {
        let step = vibration_step(intensity);
        if step == 0 {
            return self.stop(toy).await;
        }
        self.function(toy, format!("Vibrate:{step}"), LEASE.as_secs() as u8)
            .await
    }

    pub async fn stop(&self, toy: &str) -> Result<(), DeviceError> {
        self.function(toy, "Stop".into(), 0).await
    }

    /// False means a valid API response explicitly rejected Pattern as unsupported.
    /// Transport failures, invalid parameters and malformed replies remain errors.
    pub(crate) async fn pattern(&self, toy: &str, levels: &[u8]) -> Result<bool, DeviceError> {
        if toy.is_empty()
            || levels.is_empty()
            || levels.len() > 50
            || levels.iter().any(|n| *n > 20)
        {
            return Err(malformed("invalid targeted vibration pattern"));
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Pattern<'a> {
            command: &'static str,
            rule: String,
            strength: String,
            time_sec: u8,
            toy: &'a str,
            api_ver: u8,
        }
        let bytes = self
            .request(&Pattern {
                command: "Pattern",
                rule: format!("V:1;F:v;S:{}#", crate::smoothing::INTERVAL.as_millis()),
                strength: levels
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>()
                    .join(";"),
                time_sec: LEASE.as_secs() as u8,
                toy,
                api_ver: 2,
            })
            .await?;
        if matches!(decode_envelope(&bytes)?.code, 400 | 403) {
            return Ok(false);
        }
        envelope(&bytes)?;
        Ok(true)
    }

    async fn function(&self, toy: &str, action: String, time_sec: u8) -> Result<(), DeviceError> {
        // Every command has a toy ID: omission would command every paired toy.
        if toy.is_empty() {
            return Err(malformed("select a toy first"));
        }
        envelope(
            &self
                .request(&Function {
                    command: "Function",
                    action,
                    toy,
                    api_ver: 1,
                    time_sec,
                    stop_previous: 1,
                })
                .await?,
        )?;
        Ok(())
    }
}

fn network_error(error: reqwest::Error) -> DeviceError {
    tracing::debug!(%error, "Lovense HTTP request failed");
    if error.is_timeout() {
        DeviceError::Timeout
    } else {
        DeviceError::Communication(
            "Cannot reach Remote. Check Game Mode, address, port and TLS hostname.".into(),
        )
    }
}
