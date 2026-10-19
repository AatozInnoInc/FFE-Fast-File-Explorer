//! Measure first-batch and full-listing time for a real directory.
//!
//! Windows only. Prints the architecture targets beside the numbers it measured.
//! It does not pass or fail those targets.

use std::process::ExitCode;

fn main() -> ExitCode {
    run()
}

#[cfg(not(windows))]
fn run() -> ExitCode {
    eprintln!("measure_listing measures a real directory on Windows only.");
    ExitCode::from(1)
}

#[cfg(windows)]
fn run() -> ExitCode {
    use std::env;
    use std::thread;
    use std::time::{Duration, Instant};

    use fx_core::{ListingEvent, Path, Shell, TryEvent, VolumeId};

    let Some(directory) = env::args().nth(1) else {
        eprintln!("usage: measure_listing <directory>");
        return ExitCode::from(2);
    };
    let path = Path::new(directory);
    let started = Instant::now();
    let mut opened = match Shell::new().open(VolumeId::from_directory(&path), path) {
        Ok(opened) => opened,
        Err(err) => {
            eprintln!("enumeration failed to start: {err}");
            return ExitCode::from(1);
        }
    };

    let mut count = 0usize;
    let mut first_batch = None;
    let terminal = loop {
        match opened.poll() {
            Ok(TryEvent::Ready(ListingEvent::Batch(entries))) => {
                if first_batch.is_none() {
                    first_batch = Some(started.elapsed());
                }
                count += entries.len();
            }
            Ok(TryEvent::Ready(event)) => break event,
            Ok(TryEvent::Pending) => thread::sleep(Duration::from_millis(1)),
            Err(err) => {
                eprintln!("enumeration failed: {err}");
                return ExitCode::from(1);
            }
        }
    };
    let full = started.elapsed();
    println!("entries: {count}");
    match first_batch {
        Some(elapsed) => println!(
            "first batch: {} ms (target <= 250 ms; not a pass/fail gate)",
            elapsed.as_millis()
        ),
        None => println!("first batch: no batch received"),
    }
    println!(
        "full listing: {} ms (target <= 1000 ms; not a pass/fail gate)",
        full.as_millis()
    );
    println!(
        "Record this on a Windows SSD. Do not treat other machines as the architecture budget."
    );
    match terminal {
        ListingEvent::Finished => ExitCode::SUCCESS,
        other => {
            eprintln!("listing ended with {other:?}");
            ExitCode::from(1)
        }
    }
}
