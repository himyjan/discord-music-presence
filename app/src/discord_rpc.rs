use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity::Activity};
use std::sync::mpsc;

use crate::MainWindow;

const TEXT_LIMIT: usize = 128;

pub enum Command {
    Connect {
        application_id: String,
        details: String,
        state: String,
    },
    Disconnect,
}

pub fn run_worker(commands: mpsc::Receiver<Command>, ui: slint::Weak<MainWindow>) {
    let mut client = None;

    while let Ok(command) = commands.recv() {
        match command {
            Command::Connect {
                application_id,
                details,
                state,
            } => {
                set_ui_state(&ui, false, true, "Connecting to Discord desktop...".into());

                if let Err(message) = validate(&application_id, &details, &state) {
                    set_ui_state(&ui, false, false, message);
                    continue;
                }

                if let Some(mut connected) = client.take() {
                    if let Err(error) = close_client(&mut connected) {
                        eprintln!("Failed to close previous Discord IPC connection: {error}");
                    }
                }

                let mut next = DiscordIpcClient::new(application_id.trim());
                if let Err(error) = next.connect() {
                    set_ui_state(
                        &ui,
                        false,
                        false,
                        format!(
                            "Could not connect to Discord desktop. Make sure Discord is running: {error}"
                        ),
                    );
                    continue;
                }

                let mut activity = Activity::new();
                if !details.trim().is_empty() {
                    activity = activity.details(details.trim());
                }
                if !state.trim().is_empty() {
                    activity = activity.state(state.trim());
                }

                if let Err(error) = next.set_activity(activity) {
                    let close_error = next.close().err();
                    let message = match close_error {
                        Some(close_error) => {
                            format!(
                                "Could not set activity: {error}; closing IPC also failed: {close_error}"
                            )
                        }
                        None => format!("Could not set activity: {error}"),
                    };
                    set_ui_state(&ui, false, false, message);
                    continue;
                }

                client = Some(next);
                set_ui_state(&ui, true, false, "Connected to Discord desktop".into());
            }
            Command::Disconnect => {
                set_ui_state(&ui, false, true, "Disconnecting...".into());
                let result = match client.take() {
                    Some(mut connected) => close_client(&mut connected),
                    None => Ok(()),
                };
                match result {
                    Ok(()) => set_ui_state(&ui, false, false, "Disconnected".into()),
                    Err(error) => {
                        set_ui_state(
                            &ui,
                            false,
                            false,
                            format!("Disconnected with an error: {error}"),
                        );
                    }
                }
            }
        }
    }

    if let Some(mut connected) = client {
        if let Err(error) = close_client(&mut connected) {
            eprintln!("Failed to close Discord IPC during shutdown: {error}");
        }
    }
}

fn close_client(client: &mut DiscordIpcClient) -> Result<(), String> {
    let clear_result = client.clear_activity().map_err(|error| error.to_string());
    let close_result = client.close().map_err(|error| error.to_string());

    match (clear_result, close_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(clear_error), Ok(())) => Err(format!("could not clear activity: {clear_error}")),
        (Ok(()), Err(close_error)) => Err(format!("could not close IPC: {close_error}")),
        (Err(clear_error), Err(close_error)) => Err(format!(
            "could not clear activity: {clear_error}; could not close IPC: {close_error}"
        )),
    }
}

fn set_ui_state(ui: &slint::Weak<MainWindow>, connected: bool, connecting: bool, message: String) {
    let ui = ui.clone();
    if let Err(error) = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_connected(connected);
            window.set_connecting(connecting);
            window.set_connection_status(message.into());
        }
    }) {
        eprintln!("Could not update the Discord connection status: {error}");
    }
}

fn validate(application_id: &str, details: &str, state: &str) -> Result<(), String> {
    let application_id = application_id.trim();
    if application_id.is_empty() || !application_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Enter the numeric Application ID from the Discord Developer Portal.".into());
    }

    if details.chars().count() > TEXT_LIMIT || state.chars().count() > TEXT_LIMIT {
        return Err(format!(
            "Activity text must be {TEXT_LIMIT} characters or fewer."
        ));
    }

    if details.trim().is_empty() && state.trim().is_empty() {
        return Err("Enter activity details or state before connecting.".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate;

    #[test]
    fn requires_a_numeric_application_id() {
        assert!(validate("123456789012345678", "Listening to music", "").is_ok());
        assert!(validate("not-an-id", "Listening to music", "").is_err());
        assert!(validate("", "Listening to music", "").is_err());
    }

    #[test]
    fn validates_activity_text_limits_and_content() {
        assert!(validate("123456789012345678", "", "").is_err());
        assert!(validate("123456789012345678", &"x".repeat(129), "").is_err());
        assert!(validate("123456789012345678", &"x".repeat(128), "").is_ok());
    }
}
