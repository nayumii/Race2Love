//! Bounded, same-user /proc discovery. No Steam paths or Wine version assumptions.

use std::{
    fs::{self, File},
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    pub pid: u32,
    pub started: u64,
}

impl Identity {
    pub fn read(root: &Path, pid: u32) -> io::Result<(Self, u32)> {
        let stat = read_limited(&root.join(pid.to_string()).join("stat"), 4096)?;
        let (parent, started) = parse_stat(&stat).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "process exited or invalid stat")
        })?;
        Ok((Self { pid, started }, parent))
    }

    pub fn is_alive(self, root: &Path) -> bool {
        Self::read(root, self.pid).is_ok_and(|(current, _)| current == self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Game,
    Adapter,
}

struct Process {
    identity: Identity,
    parent: u32,
    role: Role,
    prefix: Option<Vec<u8>>,
}

pub(super) struct Processes {
    pub game: Identity,
    pub owners: Vec<Identity>,
}

pub(super) fn discover(root: &Path) -> io::Result<Option<Processes>> {
    let uid = fs::metadata(root.join("self"))?.uid();
    let mut processes = Vec::new();
    for entry in fs::read_dir(root)?.take(32_768) {
        let Ok(entry) = entry else { continue };
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse().ok()) else {
            continue;
        };
        if !entry.metadata().is_ok_and(|meta| meta.uid() == uid) {
            continue;
        }
        let path = entry.path();
        let Ok(cmdline) = read_limited(&path.join("cmdline"), 16 * 1024) else {
            continue;
        };
        let Some(role) = classify(&cmdline) else {
            continue;
        };
        let Ok((identity, parent)) = Identity::read(root, pid) else {
            continue;
        };
        // Only retain WINEPREFIX; never log or retain other environment values.
        let prefix = read_limited(&path.join("environ"), 64 * 1024)
            .ok()
            .and_then(|env| {
                env.split(|b| *b == 0)
                    .find_map(|item| item.strip_prefix(b"WINEPREFIX="))
                    .filter(|value| !value.is_empty())
                    .map(Vec::from)
            });
        processes.push(Process {
            identity,
            parent,
            role,
            prefix,
        });
        if processes.len() > 32 {
            return Err(io::Error::other(
                "too many LMU processes; close extra instances",
            ));
        }
    }
    let mut games = processes.iter().filter(|p| p.role == Role::Game);
    let Some(game) = games.next() else {
        return Ok(None);
    };
    if games.next().is_some() {
        return Err(io::Error::other(
            "multiple LMU games detected; close extra instances",
        ));
    }
    let mut owners = vec![game.identity];
    owners.extend(processes.iter().filter_map(|p| {
        let same_prefix = game
            .prefix
            .as_ref()
            .is_some_and(|prefix| p.prefix.as_ref() == Some(prefix));
        (p.role == Role::Adapter && (same_prefix || p.parent == game.identity.pid))
            .then_some(p.identity)
    }));
    Ok(Some(Processes {
        game: game.identity,
        owners,
    }))
}

pub(super) fn fd_paths(root: &Path, owner: Identity) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(root.join(owner.pid.to_string()).join("fd"))?.take(4096) {
        let Ok(entry) = entry else { continue };
        if entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
            .is_none()
        {
            continue;
        }
        let Ok(target) = fs::read_link(entry.path()) else {
            continue;
        };
        if is_backing_name(&target.to_string_lossy()) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_backing_name(target: &str) -> bool {
    let name = target.strip_suffix(" (deleted)").unwrap_or(target);
    name == "/memfd:wine-mapping"
        || name == "memfd:wine-mapping"
        || name.rsplit('/').next().is_some_and(|name| {
            name.strip_prefix("tmpmap-").is_some_and(|suffix| {
                suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
            })
        })
}

fn classify(cmdline: &[u8]) -> Option<Role> {
    let mut args = cmdline.split(|b| *b == 0);
    let first = basename(args.next()?);
    let executable = if matches!(
        first.as_slice(),
        b"wine" | b"wine64" | b"wine-preloader" | b"wine64-preloader"
    ) {
        basename(args.next()?)
    } else {
        first
    };
    match executable.as_slice() {
        b"le mans ultimate.exe" | b"lemansultimate.exe" => Some(Role::Game),
        b"pluginsadapter.exe" => Some(Role::Adapter),
        _ => None,
    }
}

fn basename(value: &[u8]) -> Vec<u8> {
    value
        .rsplit(|b| matches!(*b, b'/' | b'\\'))
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn parse_stat(stat: &[u8]) -> Option<(u32, u64)> {
    // comm can itself contain spaces and ')'. Fields after its final ')' start
    // at field 3; starttime is field 22, and survives executable/name changes.
    let text = std::str::from_utf8(stat).ok()?;
    let fields: Vec<_> = text[text.rfind(')')? + 1..].split_whitespace().collect();
    if matches!(*fields.first()?, "Z" | "X" | "x") {
        return None;
    }
    Some((fields.get(1)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}

fn read_limited(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "proc entry exceeds read limit",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_executable_matching_and_wine_backing_names() {
        for cmd in [
            b"Z:\\games\\Le Mans Ultimate.exe\0".as_slice(),
            b"/proton/wine64\0C:\\game\\LeMansUltimate.exe\0",
        ] {
            assert_eq!(classify(cmd), Some(Role::Game));
        }
        assert_eq!(
            classify(b"C:\\LMU\\PluginsAdapter.exe\0"),
            Some(Role::Adapter)
        );
        for cmd in [
            b"python\0Le Mans Ultimate.exe\0".as_slice(),
            b"pgrep\0-f\0LeMansUltimate.exe\0",
            b"Launch Le Mans Ultimate.exe\0",
            b"Le Mans Ultimate.exe.old\0",
        ] {
            assert_eq!(classify(cmd), None);
        }
        for name in [
            "/memfd:wine-mapping (deleted)",
            "/prefix/tmpmap-12ab34cd (deleted)",
        ] {
            assert!(is_backing_name(name));
        }
        for name in [
            "/memfd:other (deleted)",
            "/game/tmpmap-bad",
            "/data/LMU_Data",
        ] {
            assert!(!is_backing_name(name));
        }
    }

    #[test]
    fn stat_handles_parentheses_and_rejects_zombies_and_short_records() {
        let tail = "S 12 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 9876";
        assert_eq!(
            parse_stat(format!("123 (car ) driver) {tail}").as_bytes()),
            Some((12, 9876))
        );
        assert_eq!(
            parse_stat(format!("123 (car) {}", tail.replacen('S', "Z", 1)).as_bytes()),
            None
        );
        assert_eq!(parse_stat(b"123 (car) S 12"), None);
    }
}
