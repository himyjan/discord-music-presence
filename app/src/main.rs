mod discord_rpc;

use discord_rpc::{Command, run_worker};
use std::{sync::mpsc, thread};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window = MainWindow::new()?;
    let (command_tx, command_rx) = mpsc::channel();
    let ui_weak = window.as_weak();
    let worker = thread::Builder::new()
        .name("discord-rich-presence".into())
        .spawn(move || run_worker(command_rx, ui_weak))?;

    let connect_tx = command_tx.clone();
    window.on_connect_requested(move |application_id, details, state| {
        if let Err(error) = connect_tx.send(Command::Connect {
            application_id: application_id.to_string(),
            details: details.to_string(),
            state: state.to_string(),
        }) {
            eprintln!("Could not send Discord connect request: {error}");
        }
    });

    let disconnect_tx = command_tx.clone();
    window.on_disconnect_requested(move || {
        if let Err(error) = disconnect_tx.send(Command::Disconnect) {
            eprintln!("Could not send Discord disconnect request: {error}");
        }
    });

    let run_result = window.run();
    drop(window);
    drop(command_tx);

    if worker.join().is_err() {
        eprintln!("Discord Rich Presence worker stopped unexpectedly");
    }

    run_result?;
    Ok(())
}
