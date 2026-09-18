//! Thin binary entry point: delegates everything to [`urahafu::run`].

use std::process::ExitCode;

fn main() -> ExitCode {
    if let Err(err) = urahafu::run() {
        eprintln!("urahafu: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
