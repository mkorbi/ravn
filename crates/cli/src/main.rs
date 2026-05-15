//! Ravn CLI — ratatui-TUI Frontend.
//!
//! Phase-0-Skelett: tracing wird hier zentral initialisiert. Echte TUI
//! folgt in 0.8.

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() -> anyhow::Result<()> {
    init_tracing();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "ravn starting"
    );

    println!("ravn v{} — Phase 0 skeleton", env!("CARGO_PKG_VERSION"));
    Ok(())
}

/// Phase-0-Tracing: pretty fmt-Layer auf stderr, `RUST_LOG`-Env-Filter,
/// Default `info`. Strukturiertes JSON-Logging in die `events`-Tabelle
/// kommt in Phase 1 als separater Layer.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr),
        )
        .init();
}
