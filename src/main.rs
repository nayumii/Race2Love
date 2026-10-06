//! Desktop entry point and a display-free Demo smoke mode.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{error::Error, sync::Arc, time::Duration};

use race2love_core::{
    config::{Config, config_path},
    devices::MockDevice,
    runtime::RaceRuntime,
    telemetry::DemoSource,
};
use race2love_lovense::LovenseService;
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(
                "race2love=info,race2love_core=info,race2love_gui=info,race2love_lovense=info",
            )
        }))
        .try_init()
        .map_err(|error| -> Box<dyn Error> { error })?;
    let mut headless_seconds = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--demo" => {}
            "--demo-seconds" => {
                let seconds: u64 = arguments
                    .next()
                    .ok_or("--demo-seconds needs a duration")?
                    .parse()?;
                if !(1..=3_600).contains(&seconds) {
                    return Err("Demo duration must be 1..=3600 seconds".into());
                }
                headless_seconds = Some(seconds);
            }
            "--help" | "-h" => {
                println!(
                    "Race2Love\n\nUsage: race2love [--demo] [--demo-seconds SECONDS]\n\nWithout arguments, launch the native Demo window.\n--demo-seconds runs the same pipeline without a display.\nRACE2LOVE_CONFIG overrides the TOML settings path.\nRUST_LOG controls tracing output."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {argument}; use --help").into()),
        }
    }
    let (path, config, message) = match config_path() {
        Ok(path) => match Config::load(&path) {
            Ok(config) => (Some(path), config, None),
            Err(error) => {
                tracing::error!(%error, ?path, "Settings load failed; using defaults without overwriting the file");
                (Some(path), Config::default(), Some("Settings could not be loaded. Defaults are active; the original file is preserved until you save. Details are in the logs.".into()))
            }
        },
        Err(error) => {
            tracing::warn!(%error, "Settings directory unavailable");
            (None, Config::default(), Some(error.to_string()))
        }
    };
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let device = Arc::new(MockDevice::default());
    let lovense = {
        let _entered = executor.enter();
        LovenseService::spawn()
    };
    let runtime = {
        let _entered = executor.enter();
        RaceRuntime::spawn(
            Box::new(DemoSource::default()),
            device.clone(),
            config.clone(),
        )?
    };
    let result: Result<(), String> = if let Some(seconds) = headless_seconds {
        executor.block_on(async {
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            let snapshot = runtime.control.snapshot();
            println!("Demo: connected={}, gear={}, rpm={:.0}, mixed={:.3}, mock_output={:.3}, commands={}", snapshot.telemetry.connected, snapshot.telemetry.frame.as_ref().map_or(0, |frame| frame.gear), snapshot.telemetry.frame.as_ref().map_or(0.0, |frame| frame.engine_rpm), snapshot.effects.mixed, device.intensity(), device.command_count());
        });
        Ok(())
    } else {
        #[cfg(feature = "gui")]
        {
            race2love_gui::run(
                runtime.control.clone(),
                config,
                path,
                message,
                lovense.control.clone(),
                lovense.device.clone(),
                device.clone(),
            )
            .map_err(|error| format!("Could not open the native window: {error}"))
        }
        #[cfg(not(feature = "gui"))]
        {
            let _ = (config, path, message);
            Err("This build has no GUI; use --demo-seconds SECONDS or rebuild with default features.".into())
        }
    };
    executor.block_on(runtime.shutdown());
    executor.block_on(lovense.shutdown());
    tracing::info!(output = device.intensity(), "Race2Love shutdown complete");
    result.map_err(Into::into)
}
