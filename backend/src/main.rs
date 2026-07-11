use are_you_up_backend::{app, open_db};
use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Stores raw activity samples and serves derived active/idle intervals.
///
/// Precedence for every option: command-line flag, then environment
/// variable, then the built-in default (so the scripts and LaunchAgent
/// setups that export ARE_YOU_UP_* keep working unchanged).
#[derive(Parser, Debug)]
#[command(version, about)]
#[command(after_help = "Logging: RUST_LOG controls verbosity (default: info). \
Examples: RUST_LOG=debug for everything including per-request traces, \
RUST_LOG=are_you_up_backend=debug,tower_http=info for finer control.")]
struct Args {
    /// Address to bind (host:port); use the tailnet address in deployment
    #[arg(long, env = "ARE_YOU_UP_ADDR", default_value = "127.0.0.1:8080")]
    addr: String,

    /// Sqlite database file, created if missing
    #[arg(long, env = "ARE_YOU_UP_DB", default_value = "./are-you-up.db")]
    db: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let conn = open_db(&args.db);
    let listener = tokio::net::TcpListener::bind(&args.addr)
        .await
        .expect("bind --addr; the address must be free and well-formed");
    tracing::info!(addr = %args.addr, db = %args.db, "listening");
    axum::serve(listener, app(conn))
        .await
        .expect("serve fails only on unrecoverable accept errors");
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    // Note: these run with the real process environment; they assume the
    // ARE_YOU_UP_* variables are not set in the test environment.

    #[test]
    fn defaults_match_documented_values() {
        let args =
            Args::try_parse_from(["are-you-up-backend"]).expect("no args is a valid invocation");
        assert_eq!(args.addr, "127.0.0.1:8080");
        assert_eq!(args.db, "./are-you-up.db");
    }

    #[test]
    fn flags_override_defaults() {
        let args = Args::try_parse_from([
            "are-you-up-backend",
            "--addr",
            "0.0.0.0:9000",
            "--db",
            "/tmp/other.db",
        ])
        .expect("both flags are valid");
        assert_eq!(args.addr, "0.0.0.0:9000");
        assert_eq!(args.db, "/tmp/other.db");
    }

    #[test]
    fn unknown_flags_are_rejected() {
        assert!(Args::try_parse_from(["are-you-up-backend", "--port", "9000"]).is_err());
    }

    #[test]
    fn cli_definition_is_coherent() {
        // clap's self-check: catches conflicting names, broken defaults, etc.
        Args::command().debug_assert();
    }
}
