//! Versioned TOML preferences. Runtime connection state is never persisted.

use std::{
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            lovense: LovenseConfig::default(),
            effects: EffectsConfig::default(),
            output: OutputConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LovenseConfig {
    pub host: String,
    /// Unknown until Remote/Game Mode reports its port; never assume one.
    pub port: Option<u16>,
    pub automatic_reconnect: bool,
    pub request_timeout_ms: u64,
}

impl Default for LovenseConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: None,
            automatic_reconnect: true,
            request_timeout_ms: 1_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectsConfig {
    pub engine: EngineConfig,
    pub gear_shift: GearShiftConfig,
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
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            start_minimized: false,
            show_debug: false,
            refresh_hz: 30,
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
            ("global intensity", self.output.global_intensity),
            ("maximum intensity", self.output.max_intensity),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(ConfigError::Invalid(format!("{name} must be in 0..=1")));
            }
        }
        let engine = &self.effects.engine;
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
