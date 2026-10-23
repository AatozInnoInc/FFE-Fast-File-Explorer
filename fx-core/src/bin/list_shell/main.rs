//! Win32 list for one directory.
//!
//! The window calls [`fx_core::Shell`] in-process. It does not enumerate, and it
//! does not use a C ABI. On a cache hit the owner-data count is set, and the
//! window paints, before the first `poll`.

mod columns;
mod commands;
#[cfg(windows)]
mod draw;
#[cfg(windows)]
mod file_ops;
mod icons;
mod images;
mod nav;
mod places;
mod query;
mod rows;
mod timeout;
mod tree;

#[cfg(windows)]
mod window;

use std::process::ExitCode;

fn main() -> ExitCode {
    run()
}

#[cfg(not(windows))]
fn run() -> ExitCode {
    eprintln!("list_shell is the Win32 SysListView32 shell and runs on Windows.");
    ExitCode::from(1)
}

#[cfg(windows)]
fn run() -> ExitCode {
    fx_core::dump::install_beside_exe();
    window::run()
}
