//! Read-only native telemetry diagnostic; no GUI, network or device backend.
//! cargo run -p race2love-lmu --example telemetry_probe -- 30

use std::{
    error::Error,
    thread,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn Error>> {
    let seconds = std::env::args()
        .nth(1)
        .map_or(Ok(30), |value| value.parse::<u64>())?;
    if !(1..=600).contains(&seconds) {
        return Err("duration must be 1..=600 seconds".into());
    }
    let mut source = race2love_lmu::native_source();
    println!(
        "{}: read-only probe for {seconds}s; no device output",
        source.name()
    );
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut next_connect = Instant::now();
    let mut next_report = Instant::now();
    let mut last_error = None;
    let mut latest = None;
    let mut frames = 0;
    while Instant::now() < deadline {
        let now = Instant::now();
        if !source.is_connected() && now >= next_connect {
            next_connect = now + Duration::from_secs(1);
            match source.connect() {
                Ok(()) => println!("Mapping connected"),
                Err(error) => report_error(&mut last_error, error.to_string()),
            }
        }
        if source.is_connected() {
            match source.read_frame() {
                Ok(Some(frame)) => {
                    frames += 1;
                    latest = Some(frame);
                    last_error = None;
                }
                Ok(None) => {}
                Err(error) => {
                    latest = None;
                    report_error(&mut last_error, error.to_string());
                }
            }
        }
        if now >= next_report {
            next_report = now + Duration::from_secs(1);
            if let Some(frame) = latest.as_ref().filter(|frame| {
                now.saturating_duration_since(frame.timestamp) < Duration::from_millis(250)
            }) {
                println!(
                    "{frames} fresh frames/s | {:.1} km/h | {:.0}/{:.0} RPM | gear {} | throttle {:.0}% brake {:.0}% | {} | {}",
                    frame.speed_mps * 3.6,
                    frame.engine_rpm,
                    frame.engine_max_rpm,
                    frame.gear,
                    frame.throttle * 100.0,
                    frame.brake * 100.0,
                    frame.session.as_deref().unwrap_or("unknown session"),
                    frame.car.as_deref().unwrap_or("unknown car")
                );
            } else {
                println!("Waiting for advancing player telemetry ({frames} fresh frames/s)");
            }
            frames = 0;
        }
        thread::sleep(Duration::from_secs_f64(1.0 / 60.0));
    }
    source.disconnect();
    println!("Probe disconnected");
    Ok(())
}

fn report_error(previous: &mut Option<String>, error: String) {
    if previous.as_ref() != Some(&error) {
        println!("{error}");
        *previous = Some(error);
    }
}
