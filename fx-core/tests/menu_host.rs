//! The menu helper returns no verbs. It does not load a shell extension.

use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use fx_core::helper::HelperSession;

#[test]
fn helper_returns_no_verbs() {
    let command = Command::new(env!("CARGO_BIN_EXE_menu_host"));
    let session = HelperSession::spawn(command, r"C:\docs", &[r"C:\docs\a.txt".to_string()])
        .expect("menu_host spawns");
    let started = Instant::now();
    loop {
        session.supervise(Instant::now());
        if let Some(verbs) = session.try_recv() {
            assert!(verbs.is_empty());
            return;
        }
        if started.elapsed() > Duration::from_secs(2) {
            panic!("menu_host did not answer");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
