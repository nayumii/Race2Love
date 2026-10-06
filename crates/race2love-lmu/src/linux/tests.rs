use super::*;
use crate::{LmuSource, tests::fixture};
use race2love_core::{
    config::Config,
    devices::MockDevice,
    runtime::{RaceRuntime, StopReason},
    telemetry::TelemetrySource,
};
use std::{
    os::{fd::AsRawFd, unix::fs::symlink},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

static NEXT: AtomicU32 = AtomicU32::new(1);

struct ProcFixture {
    root: PathBuf,
    next: u32,
}

impl ProcFixture {
    fn new() -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("race2love-proc-{}-{id}", std::process::id()));
        fs::create_dir_all(root.join("self")).unwrap();
        Self { root, next: 0 }
    }

    fn process(&self, pid: u32, parent: u32, started: u64, cmd: &[u8], prefix: &str) {
        let path = self.root.join(pid.to_string());
        fs::create_dir_all(path.join("fd")).unwrap();
        fs::write(path.join("cmdline"), cmd).unwrap();
        fs::write(path.join("environ"), format!("WINEPREFIX={prefix}\0")).unwrap();
        self.stat(pid, parent, started);
    }

    fn stat(&self, pid: u32, parent: u32, started: u64) {
        fs::write(self.root.join(pid.to_string()).join("stat"),
            format!("{pid} (Le Mans ) Ultimate) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 {started}\n")).unwrap();
    }

    fn backing(&mut self, pid: u32, fd: u32, bytes: &[u8]) -> File {
        self.next += 1;
        let path = self.root.join(format!("tmpmap-{:08x}", self.next));
        fs::write(&path, bytes).unwrap();
        let link = self
            .root
            .join(pid.to_string())
            .join("fd")
            .join(fd.to_string());
        let _ = fs::remove_file(&link);
        symlink(&path, link).unwrap();
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap()
    }

    fn reader(&self) -> LinuxReader {
        LinuxReader {
            root: self.root.clone(),
            mapping: None,
        }
    }

    fn game(&self) {
        self.process(
            100,
            1,
            123,
            b"Z:\\arbitrary-library\\Le Mans Ultimate.exe\0",
            "/dynamic/prefix",
        );
    }
}

impl Drop for ProcFixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn advance(file: &File, elapsed: f64) {
    file.write_all_at(&elapsed.to_le_bytes(), (VEHICLES + ELAPSED) as u64)
        .unwrap();
}

#[test]
fn discovery_requires_game_associates_adapter_and_validates_mapping() {
    let mut fixture = ProcFixture::new();
    fixture.process(
        101,
        1,
        1,
        b"python\0Le Mans Ultimate.exe\0",
        "/dynamic/prefix",
    );
    let mut reader = fixture.reader();
    assert!(reader.connect().is_err());
    fixture.game();
    fixture.process(102, 1, 124, b"PluginsAdapter.exe\0", "/different/prefix");
    let _unrelated = fixture.backing(102, 2, &crate::tests::fixture(0, 99.0));
    assert!(reader.connect().is_err());
    fixture.process(103, 100, 125, b"PluginsAdapter.exe\0", "/dynamic/prefix");
    let _short = fixture.backing(103, 1, &[0; 8]);
    let _large = fixture.backing(103, 2, &vec![0; MAX_BACKING_SIZE as usize + 1]);
    let _invalid = fixture.backing(103, 3, &vec![0; ALLOCATION_SIZE]);
    let writer = fixture.backing(103, 4, &crate::tests::fixture(0, 1.0));
    let mut source = LmuSource::new(reader);
    source.connect().unwrap();
    assert_eq!(source.reader.mapping.as_ref().unwrap().owner.pid, 103);
    assert!(source.read_frame().unwrap().is_none());
    advance(&writer, 2.0);
    let frame = source.read_frame().unwrap().unwrap();
    assert_eq!(
        (frame.engine_rpm, frame.gear, frame.speed_mps),
        (7000.0, 4, 13.0)
    );
    assert!(source.read_frame().unwrap().is_none());
    fs::remove_dir_all(fixture.root.join("100")).unwrap();
    assert!(source.read_frame().is_err());
    assert!(!source.is_connected());
    // The adapter and our owned file still exist, but the game is gone.
    assert!(source.connect().is_err());
}

#[test]
fn pid_reuse_descriptor_replacement_and_truncation_disconnect() {
    let mut fixture = ProcFixture::new();
    fixture.game();
    let old = fixture.backing(100, 8, &crate::tests::fixture(0, 1.0));
    let mut source = LmuSource::new(fixture.reader());
    source.connect().unwrap();
    fixture.stat(100, 1, 999);
    assert!(matches!(
        source.read_frame(),
        Err(TelemetryError::Disconnected)
    ));
    source.connect().unwrap();
    let replacement = fixture.backing(100, 8, &crate::tests::fixture(0, 2.0));
    assert!(source.read_frame().is_err());
    assert!(!source.is_connected());
    source.connect().unwrap();
    replacement.set_len(8).unwrap();
    assert!(source.read_frame().is_err());
    assert!(!source.is_connected());
    assert!(source.connect().is_err());
    assert_eq!(old.metadata().unwrap().len(), ALLOCATION_SIZE as u64);
}

#[test]
fn disappearing_adapter_and_ambiguous_game_instances_fail_closed() {
    let mut fixture = ProcFixture::new();
    fixture.game();
    fixture.process(101, 100, 123, b"PluginsAdapter.exe\0", "");
    let _writer = fixture.backing(101, 8, &crate::tests::fixture(0, 1.0));
    let mut source = LmuSource::new(fixture.reader());
    source.connect().unwrap();
    fs::remove_dir_all(fixture.root.join("101")).unwrap();
    assert!(source.read_frame().is_err());
    fixture.process(102, 1, 124, b"LeMansUltimate.exe\0", "/other");
    assert!(source.connect().is_err());
}

#[test]
fn inconsistent_reads_are_skipped_and_bad_buffers_or_short_reads_are_errors() {
    let mut bytes = fixture(0, 1.0);
    let mut output = vec![42; PAYLOAD_SIZE];
    let mut calls = 0;
    let accepted = consistent_snapshot(
        |destination, offset| {
            let offset = offset as usize;
            destination.copy_from_slice(&bytes[offset..offset + destination.len()]);
            calls += 1;
            if calls == 4 {
                bytes[VEHICLES + ELAPSED..VEHICLES + ELAPSED + 8]
                    .copy_from_slice(&2.0_f64.to_le_bytes());
            }
            Ok(())
        },
        &mut output,
    )
    .unwrap();
    assert!(!accepted);
    assert!(output.iter().all(|byte| *byte == 42));
    assert!(consistent_snapshot(|_, _| panic!("must reject before reading"), &mut [0; 8]).is_err());
    assert!(
        consistent_snapshot(|_, _| Err(io::ErrorKind::UnexpectedEof.into()), &mut output).is_err()
    );
    // Player index bounds are left to the shared decoder, without an out-of-range read.
    bytes[TELEMETRY + 1] = 255;
    assert!(
        consistent_snapshot(
            |destination, offset| {
                let offset = offset as usize;
                destination.copy_from_slice(&bytes[offset..offset + destination.len()]);
                Ok(())
            },
            &mut output
        )
        .unwrap()
    );
    assert!(decoder::decode(&output, Instant::now()).is_err());
}

#[test]
fn real_memfd_can_be_read_through_proc_and_survives_truncation_safely() {
    let writer = File::from(
        rustix::fs::memfd_create("wine-mapping", rustix::fs::MemfdFlags::CLOEXEC).unwrap(),
    );
    writer.write_all_at(&fixture(5, 1.0), 0).unwrap();
    let (owner, _) = Identity::read(Path::new("/proc"), std::process::id()).unwrap();
    let path = PathBuf::from(format!("/proc/{}/fd/{}", owner.pid, writer.as_raw_fd()));
    assert!(
        proc::fd_paths(Path::new("/proc"), owner)
            .unwrap()
            .contains(&path)
    );
    let reader = File::open(&path).unwrap();
    let identity = (
        reader.metadata().unwrap().dev(),
        reader.metadata().unwrap().ino(),
    );
    let mut bytes = vec![0; PAYLOAD_SIZE];
    assert!(snapshot(&reader, &mut bytes).unwrap());
    assert_eq!(
        decoder::decode(&bytes, Instant::now())
            .unwrap()
            .unwrap()
            .vehicle_id,
        5
    );
    assert!(
        reader.write_all_at(&[0], 0).is_err(),
        "read-only open must remain read-only"
    );
    writer.set_len(8).unwrap();
    assert_eq!(
        snapshot(&reader, &mut bytes).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    drop(writer);
    // Parallel tests can reuse the just-closed descriptor number immediately.
    assert!(!fs::metadata(&path).is_ok_and(|meta| (meta.dev(), meta.ino()) == identity));
    assert!(
        reader.metadata().is_ok(),
        "our owned file outlives the original fd"
    );
}

#[test]
fn advancing_mapping_wins_over_retained_frozen_copy_without_rpm_changes() {
    let mut fixture = ProcFixture::new();
    fixture.game();
    let _frozen = fixture.backing(100, 1, &crate::tests::fixture(0, 1.0));
    let live = fixture.backing(100, 2, &crate::tests::fixture(0, 1.0));
    let writer = live.try_clone().unwrap();
    let update = std::thread::spawn(move || {
        for tick in 2..20 {
            std::thread::sleep(Duration::from_millis(10));
            advance(&writer, f64::from(tick));
        }
    });
    let mut reader = fixture.reader();
    reader.connect().unwrap();
    assert!(reader.mapping.as_ref().unwrap().path.ends_with("2"));
    update.join().unwrap();
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Linux fixture pipeline did not reach expected state");
}

#[tokio::test]
async fn linux_pipeline_stops_on_freeze_vehicle_exit_process_loss_and_reconnects() {
    let mut fixture = ProcFixture::new();
    fixture.game();
    let writer = fixture.backing(100, 2, &crate::tests::fixture(0, 1.0));
    let advancing = Arc::new(AtomicBool::new(true));
    let shutdown = Arc::new(AtomicBool::new(false));
    let writer_loop = writer.try_clone().unwrap();
    let worker = {
        let advancing = advancing.clone();
        let shutdown = shutdown.clone();
        std::thread::spawn(move || {
            let mut tick = 1.0;
            while !shutdown.load(Ordering::SeqCst) {
                if advancing.load(Ordering::SeqCst) {
                    tick += 0.02;
                    advance(&writer_loop, tick);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        })
    };
    let device = Arc::new(MockDevice::default());
    let runtime = RaceRuntime::spawn(
        Box::new(LmuSource::new(fixture.reader())),
        device.clone(),
        Config::default(),
    )
    .unwrap();
    wait_for(|| device.intensity() > 0.0).await;
    advancing.store(false, Ordering::SeqCst);
    wait_for(|| {
        runtime.control.snapshot().effects.reason == StopReason::StaleTelemetry
            && device.intensity() == 0.0
    })
    .await;
    advancing.store(true, Ordering::SeqCst);
    wait_for(|| device.intensity() > 0.0).await;
    writer.write_all_at(&[0], (TELEMETRY + 2) as u64).unwrap();
    wait_for(|| runtime.control.snapshot().telemetry.frame.is_none() && device.intensity() == 0.0)
        .await;
    writer.write_all_at(&[1], (TELEMETRY + 2) as u64).unwrap();
    wait_for(|| device.intensity() > 0.0).await;
    fs::remove_dir_all(fixture.root.join("100")).unwrap();
    wait_for(|| !runtime.control.snapshot().telemetry.connected && device.intensity() == 0.0).await;
    fixture.game();
    fixture.stat(100, 1, 456);
    // Keep the old bytes retained in our writer while replacing the producer fd.
    let new_writer = fixture.backing(100, 2, &crate::tests::fixture(0, 20.0));
    wait_for(|| runtime.control.snapshot().telemetry.connected).await;
    assert_eq!(device.intensity(), 0.0);
    for tick in 21..26 {
        advance(&new_writer, f64::from(tick));
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    wait_for(|| device.intensity() > 0.0).await;
    runtime.shutdown().await;
    assert_eq!(device.intensity(), 0.0);
    shutdown.store(true, Ordering::SeqCst);
    worker.join().unwrap();
}
