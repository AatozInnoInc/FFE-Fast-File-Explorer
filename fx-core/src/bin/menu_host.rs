//! Out-of-process helper for More options.
//!
//! Reads one menu request and answers with a heartbeat and an empty verb list.
//! This process does not load a shell context-menu extension. No verbs means
//! the window keeps its native commands.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use fx_core::helper::HostMsg;
use fx_core::watchdog::{Frame, TryDecode};

fn main() -> ExitCode {
    match serve() {
        Ok(()) => ExitCode::from(0),
        Err(()) => ExitCode::from(1),
    }
}

fn serve() -> Result<(), ()> {
    let menu = read_menu()?;
    match menu {
        HostMsg::Menu { .. } => {}
        _ => return Err(()),
    }
    let mut stdout = io::stdout().lock();
    write_msg(&mut stdout, &HostMsg::Beat)?;
    write_msg(&mut stdout, &HostMsg::Verbs(Vec::new()))?;
    Ok(())
}

fn read_menu() -> Result<HostMsg, ()> {
    let mut stdin = io::stdin().lock();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        match Frame::decode(&buf) {
            TryDecode::Error => return Err(()),
            TryDecode::Ready { frame, .. } => return HostMsg::decode(&frame).ok_or(()),
            TryDecode::Pending => {}
        }
        let n = stdin.read(&mut tmp).map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

fn write_msg(stdout: &mut impl Write, msg: &HostMsg) -> Result<(), ()> {
    let bytes = msg.encode().map_err(|_| ())?;
    stdout.write_all(&bytes).map_err(|_| ())?;
    stdout.flush().map_err(|_| ())
}
