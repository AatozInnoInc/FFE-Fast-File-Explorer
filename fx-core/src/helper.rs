//! Out-of-process helper session for More options.
//!
//! The helper speaks [`crate::watchdog::Frame`] messages. A silence longer than
//! the existing network timeout means the supervisor should stop it. This
//! module does not load a shell context-menu extension. Dropping a session
//! kills the child and does not join the reader.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Instant;

use crate::enumerate::DEFAULT_NETWORK_TIMEOUT;
use crate::watchdog::{Frame, FrameTooLarge, TryDecode, Watch};

const KIND_BEAT: u8 = 1;
const KIND_MENU: u8 = 2;
const KIND_VERBS: u8 = 3;

/// One message inside a [`Frame`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostMsg {
    Beat,
    Menu {
        directory: String,
        paths: Vec<String>,
    },
    Verbs(Vec<String>),
}

impl HostMsg {
    /// Length-prefixed bytes. `Err` when a field contains a NUL or the payload
    /// is above [`crate::watchdog::MAX_FRAME_LEN`].
    pub fn encode(&self) -> Result<Vec<u8>, FrameTooLarge> {
        let payload = match self {
            HostMsg::Beat => vec![KIND_BEAT],
            HostMsg::Menu { directory, paths } => {
                let mut bytes = vec![KIND_MENU];
                bytes.extend(fields(directory, paths)?);
                bytes
            }
            HostMsg::Verbs(labels) => {
                let mut bytes = vec![KIND_VERBS];
                if !labels.is_empty() {
                    bytes.extend(fields(&labels[0], &labels[1..])?);
                }
                bytes
            }
        };
        Ok(Frame::new(payload)?.encode())
    }

    /// `None` when the frame is not a beat, a menu, or a verb list.
    pub fn decode(frame: &Frame) -> Option<Self> {
        let bytes = frame.bytes();
        let kind = *bytes.first()?;
        let rest = &bytes[1..];
        match kind {
            KIND_BEAT if rest.is_empty() => Some(HostMsg::Beat),
            KIND_MENU => {
                let fields = split_fields(rest)?;
                let directory = fields.first()?.clone();
                Some(HostMsg::Menu {
                    directory,
                    paths: fields.into_iter().skip(1).collect(),
                })
            }
            KIND_VERBS if rest.is_empty() => Some(HostMsg::Verbs(Vec::new())),
            KIND_VERBS => Some(HostMsg::Verbs(split_fields(rest)?)),
            _ => None,
        }
    }
}

fn fields(first: &str, rest: &[String]) -> Result<Vec<u8>, FrameTooLarge> {
    let mut out = Vec::new();
    push_field(&mut out, first)?;
    for field in rest {
        out.push(0);
        push_field(&mut out, field)?;
    }
    Ok(out)
}

fn push_field(out: &mut Vec<u8>, field: &str) -> Result<(), FrameTooLarge> {
    let too_long = out.len().saturating_add(field.len()) > crate::watchdog::MAX_FRAME_LEN;
    if field.as_bytes().contains(&0) || too_long {
        return Err(FrameTooLarge);
    }
    out.extend_from_slice(field.as_bytes());
    Ok(())
}

fn split_fields(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.is_empty() {
        return None;
    }
    let mut fields = Vec::new();
    for field in bytes.split(|byte| *byte == 0) {
        if field.is_empty() {
            return None;
        }
        fields.push(String::from_utf8(field.to_vec()).ok()?);
    }
    Some(fields)
}

struct Shared {
    watch: Watch,
    verbs: Option<Vec<String>>,
    finished: bool,
}

/// A running helper. [`HelperSession::try_recv`] does not wait.
pub struct HelperSession {
    directory: String,
    shared: Arc<Mutex<Shared>>,
    child: Arc<Mutex<Option<Child>>>,
}

impl HelperSession {
    /// Spawns `command` and sends one menu request on its stdin.
    ///
    /// `None` means the process was not created or the request does not fit in
    /// a frame. This does not wait.
    pub fn spawn(mut command: Command, directory: &str, paths: &[String]) -> Option<Self> {
        let request = HostMsg::Menu {
            directory: directory.to_string(),
            paths: paths.to_vec(),
        }
        .encode()
        .ok()?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // `CREATE_NO_WINDOW`.
            command.creation_flags(0x0800_0000);
        }
        let mut child = command.spawn().ok()?;
        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;
        let started = Instant::now();
        let shared = Arc::new(Mutex::new(Shared {
            watch: Watch::new(DEFAULT_NETWORK_TIMEOUT, started),
            verbs: None,
            finished: false,
        }));
        let child = Arc::new(Mutex::new(Some(child)));
        let worker_shared = Arc::clone(&shared);
        let spawned = thread::Builder::new()
            .name("fx-core-menu-host".to_string())
            .spawn(move || read_helper(stdin, stdout, request, worker_shared))
            .is_ok();
        if !spawned {
            if let Some(child) = lock(&child).as_mut() {
                let _ = child.kill();
            }
            return None;
        }
        Some(Self {
            directory: directory.to_string(),
            shared,
            child,
        })
    }

    /// Helper beside `current_exe`, when that file exists.
    pub fn beside_exe(directory: &str, paths: &[String]) -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        let name = if cfg!(windows) {
            "menu_host.exe"
        } else {
            "menu_host"
        };
        let path = exe.parent()?.join(name);
        if !path.is_file() {
            return None;
        }
        Self::spawn(Command::new(path), directory, paths)
    }

    pub fn directory(&self) -> &str {
        &self.directory
    }

    /// Stops the child when the silence deadline has elapsed. Does not wait.
    pub fn supervise(&self, now: Instant) {
        let stop = {
            let shared = lock(&self.shared);
            !shared.finished && shared.watch.should_stop(now)
        };
        if stop {
            self.kill_child();
        }
    }

    /// `None` while the helper is still running. Later calls repeat the verb list.
    /// An empty list means the menu stays the native commands.
    pub fn try_recv(&self) -> Option<Vec<String>> {
        let shared = lock(&self.shared);
        shared
            .finished
            .then(|| shared.verbs.clone().unwrap_or_default())
    }

    fn kill_child(&self) {
        if let Some(child) = lock(&self.child).as_mut() {
            let _ = child.kill();
        }
    }
}

impl Drop for HelperSession {
    fn drop(&mut self) {
        self.kill_child();
    }
}

fn read_helper(
    mut stdin: impl Write,
    mut stdout: impl Read,
    request: Vec<u8>,
    shared: Arc<Mutex<Shared>>,
) {
    let _ = stdin.write_all(&request);
    let _ = stdin.flush();
    drop(stdin);
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        if lock(&shared).finished {
            return;
        }
        let read = stdout.read(&mut tmp);
        let mut shared = lock(&shared);
        if shared.finished {
            return;
        }
        let chunk = match read {
            Ok(0) | Err(_) => {
                shared.finished = true;
                if shared.verbs.is_none() {
                    shared.verbs = Some(Vec::new());
                }
                return;
            }
            Ok(n) => &tmp[..n],
        };
        buf.extend_from_slice(chunk);
        loop {
            match Frame::decode(&buf) {
                TryDecode::Pending => break,
                TryDecode::Error => {
                    shared.finished = true;
                    shared.verbs = Some(Vec::new());
                    return;
                }
                TryDecode::Ready { frame, consumed } => {
                    buf.drain(..consumed);
                    match HostMsg::decode(&frame) {
                        Some(HostMsg::Beat) => shared.watch.beat(Instant::now()),
                        Some(HostMsg::Verbs(labels)) => {
                            shared.verbs = Some(labels);
                            shared.finished = true;
                            return;
                        }
                        Some(HostMsg::Menu { .. }) | None => {
                            shared.finished = true;
                            shared.verbs = Some(Vec::new());
                            return;
                        }
                    }
                }
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::time::Duration;

    fn frame_of(bytes: &[u8]) -> Frame {
        Frame::new(bytes.to_vec()).unwrap()
    }

    #[test]
    fn menu_round_trip_and_empty_verbs() {
        let menu = HostMsg::Menu {
            directory: r"C:\docs".to_string(),
            paths: vec![r"C:\docs\a.txt".to_string()],
        };
        let encoded = menu.encode().unwrap();
        let (frame, _) = match Frame::decode(&encoded) {
            TryDecode::Ready { frame, consumed } => (frame, consumed),
            other => panic!("menu frame should decode, got {other:?}"),
        };
        assert_eq!(HostMsg::decode(&frame), Some(menu));

        let beat = HostMsg::Beat.encode().unwrap();
        match Frame::decode(&beat) {
            TryDecode::Ready { frame, consumed } => {
                assert_eq!(consumed, beat.len());
                assert_eq!(HostMsg::decode(&frame), Some(HostMsg::Beat));
            }
            other => panic!("beat should decode, got {other:?}"),
        }

        let verbs = HostMsg::Verbs(Vec::new()).encode().unwrap();
        match Frame::decode(&verbs) {
            TryDecode::Ready { frame, .. } => {
                assert_eq!(HostMsg::decode(&frame), Some(HostMsg::Verbs(Vec::new())));
            }
            other => panic!("verbs should decode, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_nul_and_an_unknown_kind() {
        let menu = HostMsg::Menu {
            directory: "bad\0path".to_string(),
            paths: Vec::new(),
        };
        assert!(menu.encode().is_err());
        assert_eq!(HostMsg::decode(&frame_of(&[9])), None);
        assert_eq!(HostMsg::decode(&frame_of(&[])), None);
        assert_eq!(HostMsg::decode(&frame_of(&[KIND_BEAT, 1])), None);
    }

    #[cfg(unix)]
    #[test]
    fn silence_stops_the_child() {
        let mut command = Command::new("sleep");
        command.arg("30");
        let session = HelperSession::spawn(command, r"C:\docs", &[]).unwrap();
        session.supervise(Instant::now() + Duration::from_secs(3));
        let started = Instant::now();
        loop {
            if let Some(verbs) = session.try_recv() {
                assert!(verbs.is_empty());
                return;
            }
            if started.elapsed() > Duration::from_secs(2) {
                panic!("silent helper was not stopped");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
