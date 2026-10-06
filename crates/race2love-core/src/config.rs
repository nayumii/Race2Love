//! Versioned TOML preferences. Runtime connection state is never persisted.

use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::effects::ResponseCurve;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Could not access the configuration file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Could not read TOML configuration: {0}")]
    Decode(#[from] toml::de::Error),
    #[error("Could not write TOML configuration: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("Invalid configuration: {0}")]
    Invalid(String),
    #[error("No user configuration directory is available; set RACE2LOVE_CONFIG")]
    NoDirectory,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub lovense: LovenseConfig,
    pub effects: EffectsConfig,
    pub output: OutputConfig,
    pub ui: UiConfig,
    pub effect_profiles: BTreeMap<String, EffectsConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            lovense: LovenseConfig::default(),
            effects: EffectsConfig::default(),
            output: OutputConfig::default(),
            ui: UiConfig::default(),
            effect_profiles: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LovenseConfig {
    pub host: String,
    /// Unknown until Remote/Game Mode reports its port; never assume one.
    pub port: Option<u16>,
    pub protocol: LocalProtocol,
    pub automatic_reconnect: bool,
    pub request_timeout_ms: u64,
    pub output_mode: LovenseOutputMode,
}

impl Default for LovenseConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: None,
            protocol: LocalProtocol::Http,
            automatic_reconnect: true,
            request_timeout_ms: 1_000,
            output_mode: LovenseOutputMode::default(),
        }
    }
}

/// Backward-compatible local output choices for physical A/B comparison.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LovenseOutputMode {
    #[default]
    Vibrate,
    Pattern,
    PatternDither,
}

impl LovenseOutputMode {
    pub const ALL: [Self; 3] = [Self::Vibrate, Self::Pattern, Self::PatternDither];

    pub fn label(self) -> &'static str {
        match self {
            Self::Vibrate => "Direct Vibrate (previous)",
            Self::Pattern => "Pattern smoothing (experimental)",
            Self::PatternDither => "Pattern smoothing + dithering (experimental)",
        }
    }
}

/// Transport for a manually configured Remote endpoint. HTTPS verifies certificates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalProtocol {
    #[default]
    Http,
    Https,
}

impl LocalProtocol {
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectsConfig {
    pub engine: EngineConfig,
    pub gear_shift: GearShiftConfig,
    pub wheel_slip: SlipConfig,
    pub road: RoadConfig,
    pub impact: ImpactConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SlipConfig {
    pub enabled: bool,
    pub threshold: f32,
    pub gain: f32,
    pub max_intensity: f32,
}
impl Default for SlipConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 0.12,
            gain: 1.5,
            max_intensity: 0.6,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoadConfig {
    pub enabled: bool,
    /// Suspension travel speed threshold in m/s; this also detects bumps off kerbs.
    pub threshold_mps: f32,
    pub gain: f32,
    /// Minimum road cue while a loaded tyre is explicitly on a rumble strip.
    pub kerb_intensity: f32,
    /// High-pass vertical acceleration threshold, m/s².
    pub acceleration_threshold_mps2: f32,
    pub acceleration_gain: f32,
    pub max_intensity: f32,
}
impl Default for RoadConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold_mps: 0.25,
            gain: 1.0,
            kerb_intensity: 0.5,
            acceleration_threshold_mps2: 1.0,
            acceleration_gain: 0.12,
            max_intensity: 0.5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImpactConfig {
    pub enabled: bool,
    /// Normalized severity; LMU maps 100 m/s² to 1 after an explicit impact event.
    pub threshold: f32,
    pub intensity: f32,
}
impl Default for ImpactConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 0.15,
            intensity: 0.8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    pub enabled: bool,
    /// Fractions of the simulator's maximum RPM, not absolute RPM.
    pub start_ratio: f32,
    pub end_ratio: f32,
    pub min_intensity: f32,
    pub max_intensity: f32,
    pub curve: ResponseCurve,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            start_ratio: 0.2,
            end_ratio: 1.0,
            min_intensity: 0.05,
            max_intensity: 0.65,
            curve: ResponseCurve::Linear,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GearShiftConfig {
    pub enabled: bool,
    pub intensity: f32,
    pub attack_ms: u64,
    pub hold_ms: u64,
    pub release_ms: u64,
}

impl Default for GearShiftConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            intensity: 0.9,
            attack_ms: 10,
            hold_ms: 70,
            release_ms: 60,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    pub global_intensity: f32,
    pub max_intensity: f32,
    pub telemetry_hz: u32,
    pub effects_hz: u32,
    pub update_hz: u32,
    pub telemetry_timeout_ms: u64,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            global_intensity: 0.5,
            max_intensity: 0.75,
            telemetry_hz: 60,
            effects_hz: 60,
            update_hz: 25,
            telemetry_timeout_ms: 250,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub start_minimized: bool,
    pub show_debug: bool,
    pub refresh_hz: u32,
    pub show_graphs: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            start_minimized: false,
            show_debug: false,
            refresh_hz: 30,
            show_graphs: false,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: &str| ConfigError::Invalid(message.into());
        if self.version != 1 {
            return Err(invalid("unsupported configuration version"));
        }
        for (name, value) in [
            ("engine start", self.effects.engine.start_ratio),
            ("engine end", self.effects.engine.end_ratio),
            ("engine minimum", self.effects.engine.min_intensity),
            ("engine maximum", self.effects.engine.max_intensity),
            ("shift intensity", self.effects.gear_shift.intensity),
            ("slip threshold", self.effects.wheel_slip.threshold),
            ("slip maximum", self.effects.wheel_slip.max_intensity),
            ("road maximum", self.effects.road.max_intensity),
            ("kerb intensity", self.effects.road.kerb_intensity),
            ("impact threshold", self.effects.impact.threshold),
            ("impact intensity", self.effects.impact.intensity),
            ("global intensity", self.output.global_intensity),
            ("maximum intensity", self.output.max_intensity),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(ConfigError::Invalid(format!("{name} must be in 0..=1")));
            }
        }
        let engine = &self.effects.engine;
        for (name, value) in [
            ("slip gain", self.effects.wheel_slip.gain),
            ("road gain", self.effects.road.gain),
            (
                "road acceleration gain",
                self.effects.road.acceleration_gain,
            ),
            (
                "road acceleration threshold",
                self.effects.road.acceleration_threshold_mps2,
            ),
            ("road threshold", self.effects.road.threshold_mps),
        ] {
            if !value.is_finite() || !(0.0..=10.0).contains(&value) {
                return Err(invalid(&format!("{name} must be in 0..=10")));
            }
        }
        if self.effect_profiles.len() > 16 {
            return Err(invalid("at most 16 effect profiles are supported"));
        }
        for (name, effects) in &self.effect_profiles {
            if name.trim().is_empty() || name.len() > 48 || name.chars().any(char::is_control) {
                return Err(invalid(
                    "profile names must contain 1..=48 bytes without control characters",
                ));
            }
            // Validate each standalone effects profile through the same rules.
            Self {
                effects: effects.clone(),
                ..Self::default()
            }
            .validate()?;
        }
        if engine.start_ratio >= engine.end_ratio {
            return Err(invalid("RPM start must be below RPM end"));
        }
        if engine.min_intensity > engine.max_intensity {
            return Err(invalid("engine minimum must not exceed maximum"));
        }
        let shift = &self.effects.gear_shift;
        if [shift.attack_ms, shift.hold_ms, shift.release_ms]
            .into_iter()
            .any(|value| value > 2_000)
        {
            return Err(invalid("shift envelope segments must be at most 2000 ms"));
        }
        for rate in [
            self.output.telemetry_hz,
            self.output.effects_hz,
            self.output.update_hz,
            self.ui.refresh_hz,
        ] {
            if !(1..=120).contains(&rate) {
                return Err(invalid("refresh rates must be in 1..=120 Hz"));
            }
        }
        if !(100..=2_000).contains(&self.output.telemetry_timeout_ms) {
            return Err(invalid("telemetry timeout must be in 100..=2000 ms"));
        }
        if self.lovense.host.trim().is_empty() || self.lovense.port == Some(0) {
            return Err(invalid("device host must be set and port must be nonzero"));
        }
        if !(100..=5_000).contains(&self.lovense.request_timeout_ms) {
            return Err(invalid("API timeout must be in 100..=5000 ms"));
        }
        Ok(())
    }

    /// Missing files use defaults. Invalid files are preserved for correction.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(?path, "Configuration not found; using defaults");
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut text = String::new();
        file.take(65_537).read_to_string(&mut text)?;
        if text.len() > 65_536 {
            return Err(ConfigError::Invalid("file exceeds 64 KiB".into()));
        }
        let config: Self = toml::from_str(&text)?;
        config.validate()?;
        tracing::info!(?path, "Configuration loaded");
        Ok(config)
    }

    /// Stage in the same directory so interruption cannot leave half-written TOML.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        static NEXT_SAVE: AtomicU64 = AtomicU64::new(0);
        self.validate()?;
        let text = toml::to_string_pretty(self)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let filename = path
            .file_name()
            .ok_or_else(|| ConfigError::Invalid("path has no filename".into()))?;
        let mut temporary_name = filename.to_os_string();
        temporary_name.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            NEXT_SAVE.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = parent.join(temporary_name);
        let result = (|| -> Result<(), ConfigError> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        } else {
            tracing::info!(?path, "Configuration saved");
        }
        result
    }
}

/// Linux honors XDG; Windows uses per-user roaming AppData. No config crate needed.
pub fn config_path() -> Result<PathBuf, ConfigError> {
    if let Some(path) = env::var_os("RACE2LOVE_CONFIG").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    #[cfg(windows)]
    let directory = env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let directory = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    directory
        .map(|directory| directory.join("race2love").join("config.toml"))
        .ok_or(ConfigError::NoDirectory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_configs_keep_working_vibrate_default_and_all_output_modes_round_trip() {
        let old: Config =
            toml::from_str("version = 1\n[lovense]\nhost = '127.0.0.1'\nport = 20010\n").unwrap();
        assert_eq!(old.lovense.output_mode, LovenseOutputMode::Vibrate);
        for mode in LovenseOutputMode::ALL {
            let mut config = old.clone();
            config.lovense.output_mode = mode;
            assert_eq!(
                toml::from_str::<Config>(&toml::to_string(&config).unwrap()).unwrap(),
                config
            );
        }
    }

    #[test]
    fn effect_profiles_round_trip_and_reject_invalid_or_unbounded_data() {
        let mut config = Config::default();
        let mut effects = config.effects.clone();
        effects.road.enabled = true;
        effects.wheel_slip.gain = 2.0;
        config.effect_profiles.insert("GT3 road".into(), effects);
        config.ui.show_graphs = true;
        let decoded: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(config, decoded);
        decoded.validate().unwrap();
        config
            .effect_profiles
            .get_mut("GT3 road")
            .unwrap()
            .road
            .threshold_mps = f32::NAN;
        assert!(config.validate().is_err());
        config.effect_profiles.clear();
        for index in 0..17 {
            config
                .effect_profiles
                .insert(index.to_string(), EffectsConfig::default());
        }
        assert!(config.validate().is_err());
    }

    #[test]
    fn toml_round_trip_and_missing_fields() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        let decoded: Config = toml::from_str(&text).unwrap();
        assert_eq!(decoded, config);
        decoded.validate().unwrap();
        let partial: Config = toml::from_str("[output]\nglobal_intensity = 0.25").unwrap();
        assert_eq!(partial.output.global_intensity, 0.25);
        assert_eq!(partial.effects, config.effects);
        partial.validate().unwrap();
    }

    #[test]
    fn documented_example_matches_the_defaults() {
        let example: Config = toml::from_str(include_str!("../../../config.example.toml")).unwrap();
        example.validate().unwrap();
        assert_eq!(example, Config::default());
    }

    #[test]
    fn invalid_values_and_versions_are_rejected() {
        let mut config = Config::default();
        config.output.update_hz = 0;
        assert!(config.validate().is_err());
        config.output = OutputConfig::default();
        config.output.global_intensity = f32::NAN;
        assert!(config.validate().is_err());
        config.output = OutputConfig::default();
        config.effects.engine.end_ratio = config.effects.engine.start_ratio;
        assert!(config.validate().is_err());
        config.effects = EffectsConfig::default();
        config.version = 2;
        assert!(config.validate().is_err());
    }

    #[test]
    fn save_load_and_replace_preserve_invalid_files() {
        let path =
            env::temp_dir().join(format!("race2love-config-test-{}.toml", std::process::id()));
        let mut config = Config::default();
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        config.output.global_intensity = 0.2;
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), config);
        fs::write(&path, "this is not TOML").unwrap();
        assert!(Config::load(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "this is not TOML");
        fs::remove_file(path).unwrap();
    }
}
