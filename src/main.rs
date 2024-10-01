use std::error::Error;

use clap::Parser;

use multiplayer_game_demo_rust::{app, globals, message, server};

#[derive(Parser)]
#[command(
    about = "Networked multiplayer proof-of-concept game demo utilizing client-server architecture. Starts with GUI interface by default where players can host and join game sessions. Also capable of running in headless server-only mode."
)]
struct Cli {
    #[arg(
        long,
        help = "Starts a server only in headless mode without graphical user interface. Used for creating dedicated servers."
    )]
    server_only: bool,

    #[arg(
        short,
        long,
        require_equals = true,
        default_value_t = globals::DEFAULT_PORT,
        help = "Port number used for server in headless mode (--server-only)."
    )]
    port: u16,

    #[arg(long, help = "Enable tracing of UDP messages on console log.")]
    trace: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    if cli.trace {
        println!("Message tracing enabled.");
        message::set_trace(true);
    }

    // Application window events, rendering and GUI are in syncronous environment and async code
    // cannot be called from there. Manage Tokio runtime separately to bridge sync
    // code with async code easily.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    if cli.server_only {
        println!("Starting server in headless mode");
        rt.block_on(async {
            match server::start_server(cli.port).await {
                Ok(_) => {
                    println!("Server started successfully. Waiting for CTRL+C to shut down.");
                    match tokio::signal::ctrl_c().await {
                        Ok(_) => println!(
                            "\nCTRL+C interrupt received. Shutting down server gracefully..."
                        ),
                        Err(e) => eprintln!("Failed to listen for CTRL+C event: {}", e),
                    }
                }
                Err(e) => {
                    eprintln!("Server failed to start: {}", e);
                    std::process::exit(1);
                }
            }
        });
        Ok(())
    } else {
        app::run_app(&rt)
    }
}
