//! Loopback-only fake Remote; tests never discover or command physical devices.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use race2love_core::{
    config::{Config, EngineConfig, LocalProtocol, LovenseConfig, LovenseOutputMode},
    devices::{DeviceError, HapticDevice},
    effects::ResponseCurve,
    runtime::{RaceRuntime, StopReason},
    telemetry::{DemoSource, TelemetryError, TelemetryFrame, TelemetrySource},
};
use race2love_lovense::{
    ConnectionState, LovenseService, RemoteClient, parse_toys, vibration_step,
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::{JoinHandle, JoinSet},
    time::Instant,
};

#[derive(Clone, Default)]
struct Reply {
    status: Option<u16>,
    body: Option<String>,
    delay: Duration,
}

#[derive(Clone)]
struct Record {
    received: Instant,
    method: String,
    path: String,
    platform: String,
    body: Value,
}

struct State {
    records: Vec<Record>,
    toys: Value,
    encoded: bool,
    get: Reply,
    function: Reply,
    pattern: Reply,
    active: BTreeMap<String, ActiveOutput>,
    pattern_overlaps: usize,
}

struct ActiveOutput {
    pattern: bool,
    levels: Vec<u8>,
    started: Instant,
    until: Instant,
    interval: Duration,
}

impl ActiveOutput {
    fn level(&self, now: Instant) -> u8 {
        if now >= self.until {
            return 0;
        }
        let index =
            (now.duration_since(self.started).as_millis() / self.interval.as_millis()) as usize;
        self.levels[index.min(self.levels.len() - 1)]
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            records: vec![],
            toys: json!({
                "a": {"id":"a", "name":"lush", "status":1, "battery":60, "shortFunctionNames":["v"]},
                "b": {"id":"b", "name":"nora", "status":"1", "nickName":"Test", "battery":"70", "fullFunctionNames":["Vibrate","Rotate"]},
                "off": {"id":"off", "name":"domi", "status":0},
                "unsupported": {"id":"unsupported", "name":"unknown", "status":1, "shortFunctionNames":["r"]}
            }),
            encoded: true,
            get: Reply::default(),
            function: Reply::default(),
            pattern: Reply::default(),
            active: BTreeMap::new(),
            pattern_overlaps: 0,
        }
    }
}

struct FakeRemote {
    config: LovenseConfig,
    state: Arc<Mutex<State>>,
    task: JoinHandle<()>,
}

impl FakeRemote {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = LovenseConfig {
            port: Some(listener.local_addr().unwrap().port()),
            // Retain all previous acceptance checks against the original strategy.
            output_mode: LovenseOutputMode::Vibrate,
            ..Default::default()
        };
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            let mut requests = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let Ok((stream, _)) = connection else { break; };
                        requests.spawn(serve(stream, shared.clone()));
                    }
                    _ = requests.join_next(), if !requests.is_empty() => {}
                }
            }
        });
        Self {
            config,
            state,
            task,
        }
    }
    fn records(&self) -> Vec<Record> {
        self.state.lock().unwrap().records.clone()
    }
    fn active(&self, toy: &str) -> u8 {
        self.state
            .lock()
            .unwrap()
            .active
            .get(toy)
            .map_or(0, |output| output.level(Instant::now()))
    }
    fn vibrations(&self) -> usize {
        self.records()
            .iter()
            .filter(|r| {
                r.body["command"] == "Pattern"
                    || r.body["action"]
                        .as_str()
                        .is_some_and(|a| a.starts_with("Vibrate:"))
            })
            .count()
    }
    fn patterns(&self) -> usize {
        self.records()
            .iter()
            .filter(|record| record.body["command"] == "Pattern")
            .count()
    }
    fn queries(&self) -> usize {
        self.records()
            .iter()
            .filter(|r| r.body["command"] == "GetToys")
            .count()
    }
}

impl Drop for FakeRemote {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(mut stream: TcpStream, shared: Arc<Mutex<State>>) {
    let mut bytes = vec![];
    let header_end;
    loop {
        let mut buffer = [0_u8; 2048];
        let Ok(size) = stream.read(&mut buffer).await else {
            return;
        };
        if size == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..size]);
        if bytes.len() > 8192 {
            return;
        }
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = end + 4;
            break;
        }
    }
    let headers = String::from_utf8_lossy(&bytes[..header_end]).to_string();
    let header = |name: &str| {
        headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
    };
    let length: usize = header("content-length").unwrap().parse().unwrap();
    while bytes.len() < header_end + length {
        let mut buffer = [0_u8; 2048];
        let Ok(size) = stream.read(&mut buffer).await else {
            return;
        };
        if size == 0 {
            return;
        }
        bytes.extend_from_slice(&buffer[..size]);
        if bytes.len() > 8192 {
            return;
        }
    }
    let body: Value = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
    let mut start = headers.lines().next().unwrap().split_whitespace();
    let record = Record {
        received: Instant::now(),
        method: start.next().unwrap().into(),
        path: start.next().unwrap().into(),
        platform: header("x-platform").unwrap_or_default(),
        body: body.clone(),
    };
    let (reply, response) = {
        let mut state = shared.lock().unwrap();
        assert!(
            state.records.len() < 1000,
            "unbounded API retry/output loop"
        );
        state.records.push(record);
        if body["command"] == "GetToys" {
            let toys = if state.encoded {
                Value::String(state.toys.to_string())
            } else {
                state.toys.clone()
            };
            (
                state.get.clone(),
                json!({"code":200, "type":"OK", "data":{"toys":toys}}).to_string(),
            )
        } else {
            let pattern = body["command"] == "Pattern";
            let reply = if pattern {
                state.pattern.clone()
            } else {
                state.function.clone()
            };
            if reply.status.is_none() && reply.body.is_none() {
                let toy = body["toy"]
                    .as_str()
                    .expect("command must target one toy")
                    .to_string();
                if body["action"] == "Stop" {
                    state.active.remove(&toy);
                } else {
                    let levels = if pattern {
                        assert_eq!(body["apiVer"], 2);
                        assert_eq!(body["rule"], "V:1;F:v;S:110#");
                        let levels: Vec<u8> = body["strength"]
                            .as_str()
                            .unwrap()
                            .split(';')
                            .map(|level| level.parse().unwrap())
                            .collect();
                        assert!(!levels.is_empty() && levels.len() <= 50);
                        levels
                    } else {
                        vec![
                            body["action"]
                                .as_str()
                                .unwrap()
                                .strip_prefix("Vibrate:")
                                .unwrap()
                                .parse()
                                .unwrap(),
                        ]
                    };
                    let lease = body["timeSec"].as_u64().unwrap();
                    assert_eq!(lease, 2);
                    assert!(levels.iter().all(|level| *level <= 20));
                    let started = Instant::now();
                    if pattern
                        && state
                            .active
                            .get(&toy)
                            .is_some_and(|old| old.pattern && old.until > started)
                    {
                        state.pattern_overlaps += 1;
                    }
                    if !pattern {
                        assert_eq!(body["stopPrevious"], 1);
                    }
                    state.active.insert(
                        toy,
                        ActiveOutput {
                            pattern,
                            levels,
                            started,
                            until: started + Duration::from_secs(lease),
                            interval: Duration::from_millis(110),
                        },
                    );
                }
            }
            (reply, json!({"code":200,"type":"ok"}).to_string())
        }
    };
    tokio::time::sleep(reply.delay).await;
    let response = reply.body.unwrap_or(response);
    let status = reply.status.unwrap_or(200);
    let headers = format!(
        "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    );
    let _ = stream.write_all(headers.as_bytes()).await;
    let _ = stream.write_all(response.as_bytes()).await;
}

async fn wait_for(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fake Remote condition timed out");
}

async fn connect(remote: &FakeRemote) -> LovenseService {
    let service = LovenseService::spawn();
    service.control.connect(remote.config.clone());
    wait_for(|| service.control.snapshot().state == ConnectionState::Connected).await;
    service.control.select_toy("b".into());
    wait_for(|| service.device.is_connected()).await;
    service
}

struct ConstantSource;
impl TelemetrySource for ConstantSource {
    fn name(&self) -> &'static str {
        "Constant mock telemetry"
    }
    fn connect(&mut self) -> Result<(), TelemetryError> {
        Ok(())
    }
    fn disconnect(&mut self) {}
    fn is_connected(&self) -> bool {
        true
    }
    fn read_frame(&mut self) -> Result<Option<TelemetryFrame>, TelemetryError> {
        Ok(Some(TelemetryFrame {
            engine_rpm: 6000.0,
            engine_max_rpm: 9000.0,
            gear: 3,
            ..Default::default()
        }))
    }
}

/// Hold RPM/gear steady while publishing fresh observations. None deliberately
/// freezes the source, retaining the last observation's original timestamp.
struct ControlledSource {
    input: watch::Receiver<Option<(f32, i8)>>,
}

impl TelemetrySource for ControlledSource {
    fn name(&self) -> &'static str {
        "Controlled normalized telemetry"
    }
    fn connect(&mut self) -> Result<(), TelemetryError> {
        Ok(())
    }
    fn disconnect(&mut self) {}
    fn is_connected(&self) -> bool {
        true
    }
    fn read_frame(&mut self) -> Result<Option<TelemetryFrame>, TelemetryError> {
        Ok(self
            .input
            .borrow()
            .map(|(engine_rpm, gear)| TelemetryFrame {
                engine_rpm,
                engine_max_rpm: 10_000.0,
                gear,
                ..Default::default()
            }))
    }
}

#[test]
fn typed_discovery_rejects_malformed_fields_and_preserves_unavailable_capabilities() {
    let toys = State::default().toys;
    for value in [toys.clone(), Value::String(toys.to_string())] {
        let parsed = parse_toys(
            json!({"code":200,"type":"OK","data":{"toys":value}})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].battery, Some(60));
        assert_eq!(parsed[1].vibration, Some(true));
        assert!(!parsed[2].connected);
        assert_eq!(parsed[2].vibration, None);
        assert_eq!(parsed[3].vibration, Some(false));
    }
    for invalid in [
        json!({}),
        json!({"code":200,"type":"OK"}),
        json!({"code":400,"type":"OK"}),
        json!({"code":200,"type":"ERROR"}),
        json!({"code":200,"type":"OK","data":{"toys":"[]"}}),
    ] {
        assert!(parse_toys(invalid.to_string().as_bytes()).is_err());
    }
    for (field, value) in [
        ("status", json!(2)),
        ("status", json!(true)),
        ("id", json!("wrong")),
        ("battery", json!(101)),
        ("name", json!("")),
    ] {
        let mut invalid = toys.clone();
        invalid["a"][field] = value;
        assert!(
            parse_toys(
                json!({"code":200,"type":"OK","data":{"toys":invalid}})
                    .to_string()
                    .as_bytes()
            )
            .is_err()
        );
    }
    assert_eq!(vibration_step(f32::NAN), 0);
    assert_eq!(vibration_step(-1.0), 0);
    assert_eq!(vibration_step(9.0), 20);
    assert_eq!(vibration_step(0.499), 9);
}

#[tokio::test]
async fn client_targets_one_toy_uses_headers_finite_commands_and_expiry() {
    let remote = FakeRemote::start().await;
    let client = RemoteClient::new(&remote.config).unwrap();
    assert_eq!(client.get_toys().await.unwrap().len(), 4);
    client.vibrate("b", 0.499).await.unwrap();
    assert_eq!(remote.active("b"), 9);
    assert_eq!(remote.active("a"), 0);
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(
        remote.active("b"),
        0,
        "a crash/outage must not require a final Stop"
    );
    client.vibrate("b", 1.0).await.unwrap();
    client.stop("b").await.unwrap();
    client.vibrate("b", f32::NAN).await.unwrap();
    assert_eq!(remote.active("b"), 0);
    for record in remote.records() {
        assert_eq!(record.method, "POST");
        assert_eq!(record.path, "/command");
        assert_eq!(record.platform, "Race2Love");
        if record.body["command"] == "Function" {
            assert_eq!(record.body["toy"], "b");
            assert_eq!(record.body["apiVer"], 1);
            assert_eq!(record.body["stopPrevious"], 1);
        }
    }
}

#[tokio::test]
async fn http_errors_malformed_oversize_redirects_and_timeouts_do_not_retry() {
    let remote = FakeRemote::start().await;
    let client = RemoteClient::new(&remote.config).unwrap();
    for reply in [
        Reply {
            status: Some(503),
            ..Default::default()
        },
        Reply {
            status: Some(302),
            ..Default::default()
        },
        Reply {
            body: Some("bad json".into()),
            ..Default::default()
        },
        Reply {
            body: Some("x".repeat(65_537)),
            ..Default::default()
        },
        Reply {
            body: Some(json!({"code":400,"type":"ERROR"}).to_string()),
            ..Default::default()
        },
    ] {
        remote.state.lock().unwrap().get = reply;
        let before = remote.queries();
        assert!(client.get_toys().await.is_err());
        assert_eq!(remote.queries(), before + 1);
    }
    remote.state.lock().unwrap().get = Reply {
        delay: Duration::from_millis(400),
        ..Default::default()
    };
    let config = LovenseConfig {
        request_timeout_ms: 100,
        ..remote.config.clone()
    };
    assert!(matches!(
        RemoteClient::new(&config).unwrap().get_toys().await,
        Err(DeviceError::Timeout)
    ));
}

#[test]
fn endpoint_validation_accepts_ip_and_verified_https_without_panics() {
    let mut config = LovenseConfig::default();
    assert!(RemoteClient::new(&config).is_err());
    config.port = Some(20010);
    for host in [
        "localhost/path",
        "user@localhost",
        "https://localhost",
        "localhost?x",
        " ",
    ] {
        config.host = host.into();
        assert!(RemoteClient::new(&config).is_err());
    }
    config.host = "::1".into();
    assert!(RemoteClient::new(&config).is_ok());
    config.host = "127-0-0-1.lovense.club".into();
    config.protocol = LocalProtocol::Https;
    assert!(RemoteClient::new(&config).is_ok());
}

#[tokio::test]
async fn discovery_never_autoselects_switch_and_disconnect_stop_previous_toy() {
    let remote = FakeRemote::start().await;
    let service = LovenseService::spawn();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(remote.queries(), 0);
    service.control.connect(remote.config.clone());
    wait_for(|| service.control.snapshot().state == ConnectionState::Connected).await;
    assert!(!service.device.is_connected());
    assert_eq!(remote.records().len(), 1);
    service.control.select_toy("unsupported".into());
    wait_for(|| service.control.snapshot().error.is_some()).await;
    assert!(!service.device.is_connected());
    service.control.select_toy("b".into());
    wait_for(|| service.device.is_connected()).await;
    service.device.set_vibration(0.5).await.unwrap();
    service.control.select_toy("a".into());
    wait_for(|| {
        service.device.is_connected() && service.control.snapshot().selected.as_deref() == Some("a")
    })
    .await;
    assert_eq!(remote.active("b"), 0);
    service.device.set_vibration(0.5).await.unwrap();
    service.control.disconnect();
    wait_for(|| service.control.snapshot().state == ConnectionState::Disconnected).await;
    assert_eq!(remote.active("a"), 0);
    service.shutdown().await;
}

#[tokio::test]
async fn full_pipeline_renews_duplicates_stops_and_shuts_down() {
    let remote = FakeRemote::start().await;
    let service = connect(&remote).await;
    let runtime = RaceRuntime::spawn(
        Box::new(ConstantSource),
        service.device.clone(),
        Config::default(),
    )
    .unwrap();
    wait_for(|| {
        runtime.control.snapshot().device.connected
            && runtime.control.snapshot().controls.emergency_stopped
    })
    .await;
    assert_eq!(remote.vibrations(), 0);
    runtime.control.resume();
    wait_for(|| remote.active("b") > 0).await;
    let before = remote.vibrations();
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(
        (2..=3).contains(&(remote.vibrations() - before)),
        "unchanged intensity should renew at 500 ms, not 25 Hz"
    );
    runtime.control.emergency_stop();
    wait_for(|| remote.active("b") == 0).await;
    let count = remote.vibrations();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(remote.vibrations(), count);
    runtime.control.resume();
    wait_for(|| remote.active("b") > 0).await;
    runtime.shutdown().await;
    assert_eq!(remote.active("b"), 0);
    service.shutdown().await;
    assert_eq!(remote.records().last().unwrap().body["action"], "Stop");
}

#[tokio::test]
async fn rpm_shift_pipeline_applies_live_settings_and_clears_pulses_on_timeout() {
    let remote = FakeRemote::start().await;
    let service = connect(&remote).await;
    let (frames, input) = watch::channel(Some((5_000.0, 3)));
    let mut config = Config::default();
    config.effects.engine = EngineConfig {
        start_ratio: 0.2,
        end_ratio: 0.8,
        min_intensity: 0.0,
        max_intensity: 0.8,
        ..EngineConfig::default()
    };
    config.effects.gear_shift.attack_ms = 20;
    config.effects.gear_shift.hold_ms = 200;
    config.effects.gear_shift.release_ms = 80;
    let runtime = RaceRuntime::spawn(
        Box::new(ControlledSource { input }),
        service.device.clone(),
        config.clone(),
    )
    .unwrap();
    wait_for(|| {
        let snapshot = runtime.control.snapshot();
        snapshot.device.connected && snapshot.controls.emergency_stopped
    })
    .await;
    assert_eq!(remote.vibrations(), 0);
    runtime.control.resume();
    // Halfway through the RPM window: 0.4 mixed * 0.5 global = Lovense step 4.
    wait_for(|| remote.active("b") == 4).await;
    frames.send_replace(Some((1_000.0, 3)));
    wait_for(|| remote.active("b") == 0).await;
    frames.send_replace(Some((8_000.0, 3)));
    wait_for(|| remote.active("b") == 8).await;
    frames.send_replace(Some((5_000.0, 3)));
    wait_for(|| remote.active("b") == 4).await;
    for (curve, expected_step) in [
        (ResponseCurve::Exponential, 2),
        (ResponseCurve::Logarithmic, 5),
        (ResponseCurve::Linear, 4),
    ] {
        config.effects.engine.curve = curve;
        runtime.control.update_config(config.clone()).unwrap();
        wait_for(|| remote.active("b") == expected_step).await;
    }

    let shifts_begin = remote.records().len();
    for gear in [4, 3] {
        frames.send_replace(Some((5_000.0, gear)));
        // Up/downshifts add a higher-priority pulse, then return to engine output.
        wait_for(|| remote.active("b") == 9).await;
        wait_for(|| remote.active("b") == 4).await;
    }
    assert!(remote.records()[shifts_begin..].iter().all(|record| {
        record.body["action"]
            .as_str()
            .and_then(|action| action.strip_prefix("Vibrate:"))
            .is_none_or(|step| (4..=9).contains(&step.parse::<u8>().unwrap()))
    }));

    // Real gearbox telemetry may briefly publish neutral between forward gears.
    frames.send_replace(Some((5_000.0, 0)));
    wait_for(|| {
        runtime
            .control
            .snapshot()
            .telemetry
            .frame
            .as_ref()
            .is_some_and(|f| f.gear == 0)
    })
    .await;
    frames.send_replace(Some((5_000.0, 4)));
    wait_for(|| remote.active("b") == 9).await;
    assert!(runtime.control.snapshot().effects.levels.shift_count > 0);
    config.effects.gear_shift.enabled = false;
    runtime.control.update_config(config.clone()).unwrap();
    wait_for(|| remote.active("b") == 4).await;
    let reenabled_at = remote.records().len();
    config.effects.gear_shift.enabled = true;
    runtime.control.update_config(config.clone()).unwrap();
    tokio::time::sleep(Duration::from_millis(320)).await;
    assert_eq!(remote.active("b"), 4);
    assert!(remote.records()[reenabled_at..].iter().all(|record| {
        record.body["action"]
            .as_str()
            .is_none_or(|action| !action.starts_with("Vibrate:") || action == "Vibrate:4")
    }));

    config.output.global_intensity = 0.25;
    runtime.control.update_config(config.clone()).unwrap();
    wait_for(|| remote.active("b") == 2).await;
    config.output.global_intensity = 1.0;
    config.output.max_intensity = 0.32;
    runtime.control.update_config(config.clone()).unwrap();
    // Downward quantization must honor a ceiling that is not a native step.
    wait_for(|| remote.active("b") == 6).await;
    let capped_at = remote.records().len();
    frames.send_replace(Some((5_000.0, 5)));
    wait_for(|| runtime.control.snapshot().effects.mixed > 0.9).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(remote.active("b"), 6);
    assert!(remote.records()[capped_at..].iter().all(|record| {
        record.body["action"]
            .as_str()
            .and_then(|action| action.strip_prefix("Vibrate:"))
            .is_none_or(|step| step.parse::<u8>().unwrap() <= 6)
    }));

    config.effects.gear_shift.enabled = false;
    config.output.global_intensity = 0.0;
    runtime.control.update_config(config.clone()).unwrap();
    wait_for(|| remote.active("b") == 0).await;
    config.effects.gear_shift.enabled = true;
    config.effects.gear_shift.hold_ms = 1_000;
    config.output.global_intensity = 0.5;
    config.output.max_intensity = 0.75;
    runtime.control.update_config(config).unwrap();
    wait_for(|| remote.active("b") == 4).await;
    frames.send_replace(Some((5_000.0, 6)));
    wait_for(|| remote.active("b") == 9).await;
    frames.send_replace(None);
    // A pulse still holding must stop on the 250 ms telemetry deadline.
    wait_for(|| {
        runtime.control.snapshot().effects.reason == StopReason::StaleTelemetry
            && remote.active("b") == 0
    })
    .await;
    assert_eq!(
        remote
            .records()
            .iter()
            .rev()
            .find(|record| record.body["command"] == "Function")
            .unwrap()
            .body["action"],
        "Stop"
    );
    let recovered_at = remote.records().len();
    frames.send_replace(Some((5_000.0, 1)));
    wait_for(|| remote.active("b") == 4).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(remote.records()[recovered_at..].iter().all(|record| {
        record.body["action"]
            .as_str()
            .is_none_or(|action| !action.starts_with("Vibrate:") || action == "Vibrate:4")
    }));
    runtime.shutdown().await;
    service.shutdown().await;
    assert_eq!(remote.active("b"), 0);
    assert_eq!(remote.records().last().unwrap().body["action"], "Stop");
}

#[tokio::test]
async fn changing_patterns_respect_slot_cadence_cancel_previous_schedule_and_track_latest() {
    for mode in [LovenseOutputMode::Pattern, LovenseOutputMode::PatternDither] {
        let mut remote = FakeRemote::start().await;
        remote.config.output_mode = mode;
        let service = connect(&remote).await;
        let (frames, input) = watch::channel(Some((5_000.0, 3)));
        let mut config = Config::default();
        config.effects.engine = EngineConfig {
            start_ratio: 0.0,
            end_ratio: 1.0,
            min_intensity: 0.0,
            max_intensity: 1.0,
            curve: ResponseCurve::Linear,
            ..EngineConfig::default()
        };
        config.effects.gear_shift.enabled = false;
        config.output.global_intensity = 1.0;
        config.output.max_intensity = 1.0;
        let runtime = RaceRuntime::spawn(
            Box::new(ControlledSource { input }),
            service.device.clone(),
            config,
        )
        .unwrap();
        wait_for(|| runtime.control.snapshot().controls.emergency_stopped).await;
        runtime.control.resume();
        wait_for(|| remote.active("b") == 10).await;
        for step in 1..=60 {
            frames.send_replace(Some((5_000.0 + step as f32 * 25.0, 3)));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let final_target_at = Instant::now();
        wait_for(|| (runtime.control.snapshot().device.intensity - 0.65).abs() < 0.001).await;
        assert!(
            final_target_at.elapsed() < Duration::from_millis(220),
            "the last small target must settle without waiting for lease renewal"
        );
        let patterns: Vec<_> = remote
            .records()
            .into_iter()
            .filter(|r| r.body["command"] == "Pattern")
            .collect();
        assert!(patterns.len() >= 2, "exercise actual replacement");
        assert_eq!(
            remote.state.lock().unwrap().pattern_overlaps,
            0,
            "cancel the previous schedule explicitly; do not assume Pattern replaces it"
        );
        for pair in patterns.windows(2) {
            assert!(
                pair[1].received - pair[0].received >= Duration::from_millis(110),
                "do not restart before the next Pattern slot"
            );
        }
        let jumped_at = Instant::now();
        frames.send_replace(Some((8_000.0, 3)));
        wait_for(|| remote.active("b") == 16).await;
        assert!(
            jumped_at.elapsed() < Duration::from_millis(100),
            "sharp feedback must bypass Pattern pacing"
        );
        assert_eq!(
            remote.records().last().unwrap().body["action"],
            "Vibrate:16"
        );
        let stopped_at = Instant::now();
        runtime.control.emergency_stop();
        wait_for(|| remote.active("b") == 0).await;
        assert!(stopped_at.elapsed() < Duration::from_millis(100));
        let count = remote.patterns();
        tokio::time::sleep(Duration::from_millis(180)).await;
        assert_eq!(
            remote.patterns(),
            count,
            "no delayed target may replay after Stop"
        );
        runtime.shutdown().await;
        service.shutdown().await;
    }
}

#[tokio::test]
async fn fractional_patterns_are_targeted_deduplicated_and_expire_without_renewal() {
    let mut remote = FakeRemote::start().await;
    remote.config.output_mode = LovenseOutputMode::PatternDither;
    let service = connect(&remote).await;
    service.device.set_vibration(0.525).await.unwrap();
    assert_eq!(remote.patterns(), 1);
    assert_eq!(remote.active("a"), 0);
    let pattern = remote
        .records()
        .into_iter()
        .find(|record| record.body["command"] == "Pattern")
        .unwrap();
    let levels: Vec<u8> = pattern.body["strength"]
        .as_str()
        .unwrap()
        .split(';')
        .map(|level| level.parse().unwrap())
        .collect();
    assert_eq!(levels.len(), 19);
    assert!(levels.iter().all(|level| *level == 10 || *level == 11));
    assert_eq!(pattern.body["toy"], "b");
    assert_eq!(pattern.platform, "Race2Love");
    for _ in 0..5 {
        service.device.set_vibration(0.525).await.unwrap();
    }
    assert_eq!(
        remote.patterns(),
        1,
        "stable fractional output must not restart patterns every frame"
    );
    wait_for(|| remote.active("b") == 10).await;
    wait_for(|| remote.active("b") == 11).await;
    service.device.stop().await.unwrap();
    assert_eq!(remote.active("b"), 0);
    service.device.set_vibration(0.525).await.unwrap();
    assert_eq!(remote.active("b"), 11, "Stop resets shaping/dither history");
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(
        remote.active("b"),
        0,
        "Pattern must expire if the application stops renewing"
    );
    service.shutdown().await;
}

#[tokio::test]
async fn unsupported_pattern_falls_back_once_and_reconnect_resets_capability() {
    for code in [400, 403] {
        let mut remote = FakeRemote::start().await;
        remote.config.output_mode = LovenseOutputMode::PatternDither;
        remote.state.lock().unwrap().pattern.body =
            Some(json!({"code":code,"type":"ERROR"}).to_string());
        let service = connect(&remote).await;
        service.device.set_vibration(0.525).await.unwrap();
        assert_eq!(remote.active("b"), 10);
        assert!(service.control.snapshot().using_vibrate_fallback);
        assert_eq!(remote.patterns(), 1);
        service.device.set_vibration(0.575).await.unwrap();
        assert_eq!(remote.active("b"), 11);
        assert_eq!(remote.patterns(), 1);
        service.control.connect(remote.config.clone());
        wait_for(|| {
            service.device.is_connected() && !service.control.snapshot().using_vibrate_fallback
        })
        .await;
        assert_eq!(remote.active("b"), 0);
        service.device.set_vibration(0.525).await.unwrap();
        assert_eq!(remote.patterns(), 2);
        service.shutdown().await;
        assert_eq!(remote.active("b"), 0);
    }
}

#[tokio::test]
async fn malformed_or_invalid_pattern_responses_fail_closed_without_compatibility_retry() {
    for body in [
        "bad json".to_owned(),
        json!({"code":404,"type":"ERROR"}).to_string(),
    ] {
        let mut remote = FakeRemote::start().await;
        remote.config.output_mode = LovenseOutputMode::PatternDither;
        remote.state.lock().unwrap().pattern.body = Some(body);
        let service = connect(&remote).await;
        assert!(service.device.set_vibration(0.525).await.is_err());
        assert_eq!(remote.patterns(), 1);
        assert!(!service.control.snapshot().using_vibrate_fallback);
        assert!(!service.device.is_connected());
        assert_eq!(
            remote.active("b"),
            0,
            "a failed Pattern must stop its positive Function prelude"
        );
        service.shutdown().await;
        assert_eq!(remote.active("b"), 0);
    }
}

#[tokio::test]
async fn emergency_stop_preempts_an_accepted_slow_pattern_response() {
    let mut remote = FakeRemote::start().await;
    remote.config.output_mode = LovenseOutputMode::PatternDither;
    let service = connect(&remote).await;
    let mut config = Config::default();
    config.effects.engine.min_intensity = 0.525;
    config.effects.engine.max_intensity = 0.525;
    config.output.global_intensity = 1.0;
    remote.state.lock().unwrap().pattern.delay = Duration::from_millis(800);
    let runtime =
        RaceRuntime::spawn(Box::new(ConstantSource), service.device.clone(), config).unwrap();
    wait_for(|| runtime.control.snapshot().controls.emergency_stopped).await;
    runtime.control.resume();
    wait_for(|| remote.active("b") > 0).await;
    let stopped_at = Instant::now();
    runtime.control.emergency_stop();
    wait_for(|| remote.active("b") == 0).await;
    assert!(stopped_at.elapsed() < Duration::from_millis(200));
    let commands = remote.patterns();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(remote.patterns(), commands);
    runtime.shutdown().await;
    service.shutdown().await;
}

#[tokio::test]
async fn pattern_pipeline_forwards_a_tighter_ceiling_even_when_target_is_unchanged() {
    let mut remote = FakeRemote::start().await;
    remote.config.output_mode = LovenseOutputMode::PatternDither;
    let service = connect(&remote).await;
    let (frames, input) = watch::channel(Some((6_000.0, 3)));
    let mut config = Config::default();
    config.effects.engine.min_intensity = 0.525;
    config.effects.engine.max_intensity = 0.525;
    config.output.global_intensity = 1.0;
    config.output.max_intensity = 0.8;
    let runtime = RaceRuntime::spawn(
        Box::new(ControlledSource { input }),
        service.device.clone(),
        config.clone(),
    )
    .unwrap();
    wait_for(|| runtime.control.snapshot().controls.emergency_stopped).await;
    runtime.control.resume();
    wait_for(|| remote.patterns() > 0).await;
    wait_for(|| remote.active("b") == 11).await;
    let capped_at = remote.records().len();
    config.output.max_intensity = 0.53;
    runtime.control.update_config(config.clone()).unwrap();
    wait_for(|| {
        remote.records()[capped_at..].iter().any(|record| {
            record.body["command"] == "Pattern" || record.body["action"] == "Vibrate:10"
        })
    })
    .await;
    for record in &remote.records()[capped_at..] {
        if record.body["command"] == "Pattern" {
            assert!(
                record.body["strength"]
                    .as_str()
                    .unwrap()
                    .split(';')
                    .all(|level| level.parse::<u8>().unwrap() <= 10)
            );
        }
    }
    for _ in 0..25 {
        assert!(remote.active("b") <= 10);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!((runtime.control.snapshot().effects.intensity - 0.525).abs() < 0.0001);
    config.output.max_intensity = 0.8;
    runtime.control.update_config(config).unwrap();
    wait_for(|| remote.active("b") == 11).await;
    frames.send_replace(None);
    wait_for(|| {
        runtime.control.snapshot().effects.reason == StopReason::StaleTelemetry
            && remote.active("b") == 0
    })
    .await;
    runtime.shutdown().await;
    service.shutdown().await;
    assert_eq!(remote.active("b"), 0);
}

#[tokio::test]
async fn manual_test_works_without_telemetry_obeys_ceiling_expires_and_respects_stop() {
    let remote = FakeRemote::start().await;
    let service = connect(&remote).await;
    let mut config = Config::default();
    config.output.max_intensity = 0.13;
    let runtime = RaceRuntime::spawn(
        Box::new(DemoSource::default()),
        service.device.clone(),
        config,
    )
    .unwrap();
    wait_for(|| {
        runtime.control.snapshot().device.connected
            && runtime.control.snapshot().controls.emergency_stopped
    })
    .await;
    assert!(!runtime.control.test_vibration());
    runtime.control.set_source_enabled(false);
    runtime.control.resume();
    assert!(runtime.control.test_vibration());
    wait_for(|| remote.active("b") == 2).await;
    wait_for(|| remote.active("b") == 0).await;
    assert!(!runtime.control.snapshot().controls.source_enabled);
    assert!(
        remote
            .records()
            .iter()
            .filter_map(|r| r.body["action"].as_str())
            .filter(|a| a.starts_with("Vibrate:"))
            .all(|a| a == "Vibrate:2")
    );
    assert!(runtime.control.test_vibration());
    wait_for(|| remote.active("b") > 0).await;
    runtime.control.emergency_stop();
    wait_for(|| remote.active("b") == 0).await;
    runtime.shutdown().await;
    service.shutdown().await;
}

#[tokio::test]
async fn emergency_stop_cancels_slow_response_and_preempts_slow_discovery() {
    let remote = FakeRemote::start().await;
    let service = connect(&remote).await;
    let runtime = RaceRuntime::spawn(
        Box::new(ConstantSource),
        service.device.clone(),
        Config::default(),
    )
    .unwrap();
    wait_for(|| {
        runtime.control.snapshot().device.connected
            && runtime.control.snapshot().controls.emergency_stopped
    })
    .await;
    remote.state.lock().unwrap().function.delay = Duration::from_millis(800);
    runtime.control.resume();
    wait_for(|| remote.active("b") > 0).await;
    remote.state.lock().unwrap().function.delay = Duration::ZERO;
    let started = Instant::now();
    runtime.control.emergency_stop();
    wait_for(|| remote.active("b") == 0).await;
    assert!(started.elapsed() < Duration::from_millis(200));
    remote.state.lock().unwrap().get.delay = Duration::from_millis(800);
    let queries = remote.queries();
    wait_for(|| remote.queries() > queries).await;
    runtime.control.resume();
    runtime.control.test_vibration();
    wait_for(|| remote.active("b") > 0).await;
    let started = Instant::now();
    runtime.control.emergency_stop();
    wait_for(|| remote.active("b") == 0).await;
    assert!(started.elapsed() < Duration::from_millis(200));
    runtime.shutdown().await;
    service.shutdown().await;
}

#[tokio::test]
async fn output_failure_reconnects_safely_and_never_replays_before_resume() {
    let remote = FakeRemote::start().await;
    let service = connect(&remote).await;
    let runtime = RaceRuntime::spawn(
        Box::new(ConstantSource),
        service.device.clone(),
        Config::default(),
    )
    .unwrap();
    wait_for(|| {
        runtime.control.snapshot().device.connected
            && runtime.control.snapshot().controls.emergency_stopped
    })
    .await;
    runtime.control.resume();
    wait_for(|| remote.active("b") > 0).await;
    remote.state.lock().unwrap().function.status = Some(503);
    wait_for(|| service.control.snapshot().state == ConnectionState::Reconnecting).await;
    wait_for(|| runtime.control.snapshot().controls.emergency_stopped).await;
    let vibrations = remote.vibrations();
    remote.state.lock().unwrap().function.status = None;
    wait_for(|| {
        service.device.is_connected()
            && service.control.snapshot().state == ConnectionState::Connected
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(remote.active("b"), 0);
    assert_eq!(remote.vibrations(), vibrations);
    runtime.control.resume();
    wait_for(|| remote.active("b") > 0).await;
    runtime.shutdown().await;
    service.shutdown().await;
}

#[tokio::test]
async fn toy_loss_and_return_change_epoch_and_stop_output() {
    let remote = FakeRemote::start().await;
    let service = connect(&remote).await;
    let runtime = RaceRuntime::spawn(
        Box::new(ConstantSource),
        service.device.clone(),
        Config::default(),
    )
    .unwrap();
    wait_for(|| {
        runtime.control.snapshot().device.connected
            && runtime.control.snapshot().controls.emergency_stopped
    })
    .await;
    runtime.control.resume();
    wait_for(|| remote.active("b") > 0).await;
    let epoch = service.device.connection_epoch();
    remote.state.lock().unwrap().toys["b"]["status"] = json!(0);
    wait_for(|| !service.device.is_connected()).await;
    wait_for(|| remote.active("b") == 0 && runtime.control.snapshot().controls.emergency_stopped)
        .await;
    remote.state.lock().unwrap().toys["b"]["status"] = json!(1);
    wait_for(|| service.device.is_connected()).await;
    assert!(service.device.connection_epoch() > epoch);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(remote.active("b"), 0);
    runtime.shutdown().await;
    service.shutdown().await;
}

#[tokio::test]
async fn disabled_reconnect_stops_requests_after_failure() {
    let remote = FakeRemote::start().await;
    let service = LovenseService::spawn();
    remote.state.lock().unwrap().get.status = Some(503);
    service.control.connect(LovenseConfig {
        automatic_reconnect: false,
        ..remote.config.clone()
    });
    wait_for(|| service.control.snapshot().state == ConnectionState::Exhausted).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(remote.queries(), 1);
    service.shutdown().await;
}

#[tokio::test]
async fn permanent_output_and_stop_failure_do_not_form_a_stop_loop() {
    let remote = FakeRemote::start().await;
    let service = connect(&remote).await;
    service.control.connect(LovenseConfig {
        automatic_reconnect: false,
        ..remote.config.clone()
    });
    wait_for(|| service.control.snapshot().state == ConnectionState::Connected).await;
    service.control.select_toy("b".into());
    wait_for(|| service.device.is_connected()).await;
    let runtime = RaceRuntime::spawn(
        Box::new(ConstantSource),
        service.device.clone(),
        Config::default(),
    )
    .unwrap();
    wait_for(|| {
        runtime.control.snapshot().device.connected
            && runtime.control.snapshot().controls.emergency_stopped
    })
    .await;
    remote.state.lock().unwrap().function.status = Some(503);
    runtime.control.resume();
    wait_for(|| service.control.snapshot().state == ConnectionState::Exhausted).await;
    wait_for(|| runtime.control.snapshot().controls.emergency_stopped).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let count = remote.records().len();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(remote.records().len(), count);
    assert!(
        count < 20,
        "Stop failures must not repeatedly invalidate the connection epoch"
    );
    runtime.shutdown().await;
    service.shutdown().await;
}
