//! Minimal Tokiori command line interface (P0).
//!
//! The commands here are a thin shell over `tokiori-model` and `tokiori-pg`.
//! They are deliberately not the stable CLI contract planned for P4: output
//! layout may change, but the error codes are already stable identifiers.

mod cli;

use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match cli::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            err.report();
            ExitCode::FAILURE
        }
    }
}
