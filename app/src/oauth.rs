use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::{MainWindow, oauth_protocol};

const SERVICE: &str = "app.musicpresence.discord";
const ACCOUNT: &str = "oauth2-identity-session";
const DEFAULT_CLIENT_ID: &str = "1571063796418744412";
const REDIRECT_URI: &str = "kizzy://oauth2/callback";
const AUTHORIZE_URL: &str = "https://discord.com/oauth2/authorize";
const TOKEN_URL: &str = "https://discord.com/api/v10/oauth2/token";
const USER_URL: &str = "https://discord.com/api/v10/users/@me";
const REVOKE_URL: &str = "https://discord.com/api/v10/oauth2/token/revoke";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_PROFILE_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PROFILE_IMAGE_DIMENSION: u32 = 2048;

pub fn open_authorization_url(url: &str) -> Result<(), String> {
    webbrowser::open(url).map(|_| ()).map_err(|error| {
        format!("Could not open the system browser automatically: {error}")
    })
}

#[derive(Serialize, Deserialize)]
struct Session {
    client_id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
    username: String,
    #[serde(default)]
    profile: Option<DiscordUser>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(refresh_token) = &mut self.refresh_token {
            refresh_token.zeroize();
        }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Clone, Deserialize, Serialize)]
struct DiscordUser {
    id: String,
    username: String,
    #[serde(default)]
    global_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    banner: Option<String>,
    #[serde(default)]
    accent_color: Option<u32>,
    /// Undocumented but commonly returned by /users/@me; may be absent.
    #[serde(default)]
    bio: Option<String>,
}

struct DecodedImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Default)]
struct ProfileImages {
    avatar: Option<DecodedImage>,
    banner: Option<DecodedImage>,
    status: String,
}

pub enum Command {
    Login(String),
    Logout,
}

pub fn run_worker(
    commands: Receiver<Command>,
    ui: slint::Weak<MainWindow>,
    stopping: Arc<AtomicBool>,
    cancel_login: Arc<AtomicBool>,
) {
    restore_session(&ui);

    while let Ok(command) = commands.recv() {
        match command {
            Command::Login(client_id) => {
                set_ui(
                    &ui,
                    false,
                    true,
                    "Starting Discord OAuth2 login...".into(),
                    None,
                );
                match login(&client_id, &stopping, &cancel_login, &ui) {
                    Ok((session, images)) => {
                        let username = session.username.clone();
                        match save_session(&session) {
                            Ok(()) => {
                                set_profile(&ui, session.profile.clone(), images);
                                set_ui(
                                    &ui,
                                    true,
                                    false,
                                    format!("Signed in to Discord as {username}. OAuth is for identity only.").into(),
                                    Some(true),
                                );
                            }
                            Err(error) => set_ui(
                                &ui,
                                false,
                                false,
                                format!("Discord authorized the app, but the session could not be saved: {error}").into(),
                                None,
                            ),
                        }
                    }
                    Err(error) => set_ui(&ui, false, false, error.into(), None),
                }
            }
            Command::Logout => {
                set_ui(&ui, false, true, "Signing out of Discord...".into(), None);
                let result = logout();
                match result {
                    Ok(status) => {
                        set_profile(&ui, None, ProfileImages::default());
                        set_ui(&ui, false, false, status.into(), Some(false));
                    }
                    Err(error) => set_ui(&ui, false, false, error.into(), None),
                }
            }
        }
    }
}

fn login(
    client_id: &str,
    stopping: &AtomicBool,
    cancel_login: &AtomicBool,
    ui: &slint::Weak<MainWindow>,
) -> Result<(Session, ProfileImages), String> {
    set_authorization_url(ui, String::new());
    let client_id = if client_id.trim().is_empty() {
        DEFAULT_CLIENT_ID
    } else {
        client_id.trim()
    };
    if client_id.is_empty() || !client_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Enter the numeric Discord Application ID first.".into());
    }

    let listener = TcpListener::bind("127.0.0.1:49153")
        .map_err(|error| format!("Could not open OAuth callback on 127.0.0.1:49153: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Could not prepare OAuth callback listener: {error}"))?;
    oauth_protocol::register_handler()?;

    let state = random_hex(32)?;
    let verifier = random_hex(32)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut authorize_url = Url::parse(AUTHORIZE_URL)
        .map_err(|error| format!("Invalid Discord authorization URL: {error}"))?;
    authorize_url
        .query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", "identify")
        .append_pair("state", &state)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &challenge);

    let callback = receive_callback(
        &listener,
        &state,
        authorize_url.as_str(),
        stopping,
        cancel_login,
        ui,
    );
    set_authorization_url(ui, String::new());
    let callback = callback?;
    if stopping.load(Ordering::Relaxed) || cancel_login.load(Ordering::Relaxed) {
        return Err("Discord OAuth2 login cancelled.".into());
    }
    let code = callback?;

    let response: TokenResponse = post_form(
        TOKEN_URL,
        &[
            ("client_id", client_id),
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier.as_str()),
        ],
    )?;
    let access_token = Zeroizing::new(response.access_token);
    let user = get_current_user(&access_token)?;
    let username = profile_display_name(&user);
    let images = fetch_profile_images(&user);

    Ok((
        Session {
            client_id: client_id.to_owned(),
            access_token: access_token.to_string(),
            refresh_token: response.refresh_token,
            expires_at: unix_seconds().saturating_add(response.expires_in),
            username,
            profile: Some(user),
        },
        images,
    ))
}

fn receive_callback(
    listener: &TcpListener,
    expected_state: &str,
    authorize_url: &str,
    stopping: &AtomicBool,
    cancel_login: &AtomicBool,
    ui: &slint::Weak<MainWindow>,
) -> Result<Result<String, String>, String> {
    if stopping.load(Ordering::Relaxed) || cancel_login.load(Ordering::Relaxed) {
        return Ok(Err("Discord OAuth2 login cancelled.".into()));
    }
    set_authorization_url(ui, authorize_url.to_owned());
    match open_authorization_url(authorize_url) {
        Ok(()) => set_ui(
            ui,
            false,
            true,
            "Browser opened. Waiting for Discord authorization...".into(),
            None,
        ),
        Err(error) => set_ui(
            ui,
            false,
            true,
            format!(
                "Could not open the browser automatically: {error}. Use the sign-in link below."
            ),
            None,
        ),
    }

    let deadline = std::time::Instant::now() + CALLBACK_TIMEOUT;
    loop {
        if stopping.load(Ordering::Relaxed) || cancel_login.load(Ordering::Relaxed) {
            return Ok(Err("Discord OAuth2 login cancelled.".into()));
        }
        if std::time::Instant::now() >= deadline {
            return Ok(Err("Discord OAuth2 login timed out.".into()));
        }

        match listener.accept() {
            Ok((mut stream, _)) => match read_callback_request(&mut stream) {
                Ok(Some(path)) => {
                    if path != "/oauth/relay" && !path.starts_with("/oauth/relay?") {
                        respond(&mut stream, 404, "Not found.")?;
                        continue;
                    }
                    let result = parse_relay_callback(&path, expected_state);
                    let (status, message) = match &result {
                        Ok(_) => (200, "Discord login complete. You can close this tab."),
                        Err(_) => (400, "Discord login failed. Return to the app for details."),
                    };
                    respond(&mut stream, status, message)?;
                    return Ok(result);
                }
                Ok(None) => {
                    respond(&mut stream, 400, "Invalid OAuth callback.")?;
                }
                Err(error) => {
                    let _ = respond(&mut stream, 400, "Invalid OAuth callback.");
                    return Err(error);
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(format!("OAuth callback listener failed: {error}")),
        }
    }
}

fn read_callback_request(stream: &mut TcpStream) -> Result<Option<String>, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| format!("Could not set OAuth callback timeout: {error}"))?;
    let mut request = Vec::with_capacity(1024);
    let mut buffer = [0; 1024];
    while request.len() <= 8192 {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| format!("Could not read OAuth callback: {error}"))?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    if request.len() > 8192 {
        return Err("OAuth callback request was too large.".into());
    }

    let request = String::from_utf8_lossy(&request);
    let Some(request_line) = request.lines().next() else {
        return Ok(None);
    };
    let mut parts = request_line.split_whitespace();
    if parts.next() != Some("GET") {
        return Ok(None);
    }
    Ok(parts.next().map(str::to_owned))
}

fn parse_callback(path: &str, expected_state: &str) -> Result<String, String> {
    let url = Url::parse(&format!("http://127.0.0.1{path}"))
        .map_err(|_| "Discord returned an invalid OAuth callback.".to_string())?;
    let mut code = None;
    let mut state = None;
    let mut issuer = None;
    let mut oauth_error = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "iss" => issuer = Some(value.into_owned()),
            "error" => oauth_error = Some(value.into_owned()),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Err("Discord OAuth state did not match; login was rejected.".into());
    }
    if issuer.as_deref() != Some("https://discord.com") {
        return Err("OAuth response issuer was missing or invalid.".into());
    }
    if let Some(error) = oauth_error {
        return Err(if error == "access_denied" {
            "Discord login was cancelled.".into()
        } else {
            format!("Discord OAuth authorization failed: {error}")
        });
    }
    code.filter(|value| !value.is_empty())
        .ok_or_else(|| "Discord did not return an authorization code.".into())
}

fn parse_relay_callback(path: &str, expected_state: &str) -> Result<String, String> {
    let relay_url = Url::parse(&format!("http://127.0.0.1{path}"))
        .map_err(|_| "The OAuth callback relay request was invalid.".to_string())?;
    if relay_url.path() != "/oauth/relay" {
        return Err("The OAuth callback relay path was invalid.".into());
    }

    let callback_uri = relay_url
        .query_pairs()
        .find_map(|(key, value)| (key == "uri").then(|| value.into_owned()))
        .ok_or_else(|| "The OAuth callback relay URL was missing.".to_string())?;
    let callback_url = Url::parse(&callback_uri)
        .map_err(|_| "Discord returned an invalid OAuth callback URL.".to_string())?;
    if callback_url.scheme() != "kizzy"
        || callback_url.host_str() != Some("oauth2")
        || callback_url.path() != "/callback"
    {
        return Err("Discord returned an unexpected OAuth callback URL.".into());
    }

    let callback_path = format!(
        "/oauth/callback?{}",
        callback_url.query().unwrap_or_default()
    );
    parse_callback(&callback_path, expected_state)
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<TokenResponse, String> {
    let response = agent()
        .post(url)
        .set("Accept", "application/json")
        .send_form(form)
        .map_err(|error| http_error("OAuth token request", error))?;
    let body = Zeroizing::new(
        response
            .into_string()
            .map_err(|error| format!("Could not read Discord OAuth token response: {error}"))?,
    );
    serde_json::from_str(&body)
        .map_err(|error| format!("Discord returned an invalid OAuth token response: {error}"))
}

fn get_current_user(token: &str) -> Result<DiscordUser, String> {
    let response = agent()
        .get(USER_URL)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .call()
        .map_err(|error| http_error("Discord identity request", error))?;
    let body = response
        .into_string()
        .map_err(|error| format!("Could not read Discord identity response: {error}"))?;
    serde_json::from_str(&body)
        .map_err(|error| format!("Discord returned an invalid identity response: {error}"))
}

fn fetch_profile_images(profile: &DiscordUser) -> ProfileImages {
    let mut errors = Vec::new();
    let avatar = match fetch_profile_image(
        "avatars",
        profile.id.as_str(),
        profile.avatar.as_deref(),
        128,
    ) {
        Ok(image) => image,
        Err(error) => {
            errors.push(format!("Avatar unavailable: {error}"));
            None
        }
    };
    let banner = match fetch_profile_image(
        "banners",
        profile.id.as_str(),
        profile.banner.as_deref(),
        512,
    ) {
        Ok(image) => image,
        Err(error) => {
            errors.push(format!("Banner unavailable: {error}"));
            None
        }
    };
    let status = if !errors.is_empty() {
        errors.join(" ")
    } else if avatar.is_none() && banner.is_none() {
        "No custom avatar or profile banner is set.".into()
    } else {
        "Profile artwork loaded from Discord.".into()
    };
    ProfileImages {
        avatar,
        banner,
        status,
    }
}

fn fetch_profile_image(
    resource: &str,
    user_id: &str,
    hash: Option<&str>,
    size: u16,
) -> Result<Option<DecodedImage>, String> {
    let Some(hash) = hash else {
        return Ok(None);
    };
    if !user_id.bytes().all(|byte| byte.is_ascii_digit())
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("Discord returned an invalid image identifier.".into());
    }
    let url = format!("https://cdn.discordapp.com/{resource}/{user_id}/{hash}.png?size={size}");
    let response = agent()
        .get(&url)
        .call()
        .map_err(|error| http_error("Discord profile image request", error))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_PROFILE_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read Discord profile image: {error}"))?;
    if bytes.len() as u64 > MAX_PROFILE_IMAGE_BYTES {
        return Err("Discord profile image exceeded the 4 MB limit.".into());
    }
    decode_png(&bytes).map(Some)
}

fn decode_png(bytes: &[u8]) -> Result<DecodedImage, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("Discord profile image is not a valid PNG: {error}"))?;
    let dimensions = reader.info();
    if dimensions.width == 0
        || dimensions.height == 0
        || dimensions.width > MAX_PROFILE_IMAGE_DIMENSION
        || dimensions.height > MAX_PROFILE_IMAGE_DIMENSION
    {
        return Err("Discord profile image dimensions are outside the 2048×2048 limit.".into());
    }
    let output_size = reader
        .output_buffer_size()
        .ok_or_else(|| "Discord profile image is too large to decode.".to_string())?;
    let mut decoded = vec![0; output_size];
    let info = reader
        .next_frame(&mut decoded)
        .map_err(|error| format!("Could not decode Discord profile image: {error}"))?;
    let pixels = &decoded[..info.buffer_size()];
    let pixel_count = (info.width as usize)
        .checked_mul(info.height as usize)
        .ok_or_else(|| "Discord profile image dimensions overflowed.".to_string())?;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(pixels),
        png::ColorType::Rgb => {
            for pixel in pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &gray in pixels {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in pixels.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => {
            return Err("Discord profile image used an unsupported indexed PNG.".into());
        }
    }
    if rgba.len() != pixel_count * 4 {
        return Err("Discord profile image has an invalid pixel buffer.".into());
    }
    Ok(DecodedImage {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn profile_display_name(profile: &DiscordUser) -> String {
    profile
        .global_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&profile.username)
        .to_owned()
}

fn profile_initials(name: &str) -> String {
    let initials = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect::<String>();
    if initials.is_empty() {
        "?".into()
    } else {
        initials
    }
}

fn set_profile(ui: &slint::Weak<MainWindow>, profile: Option<DiscordUser>, images: ProfileImages) {
    let ui = ui.clone();
    if let Err(error) = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            if let Some(profile) = profile {
                let display_name = profile_display_name(&profile);
                let initials = profile_initials(&display_name);
                window.set_oauth_profile_loaded(true);
                window.set_oauth_display_name(display_name.into());
                window.set_oauth_username(format!("@{}", profile.username).into());
                window.set_oauth_user_id(profile.id.into());
                window.set_oauth_initials(initials.into());
                let bio = profile
                    .bio
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("No about-me text is set on this account.");
                window.set_oauth_bio(bio.into());
                window.set_oauth_accent_color(profile.accent_color.map_or(
                    slint::Color::from_rgb_u8(35, 36, 40),
                    |color| {
                        slint::Color::from_rgb_u8(
                            ((color >> 16) & 0xff) as u8,
                            ((color >> 8) & 0xff) as u8,
                            (color & 0xff) as u8,
                        )
                    },
                ));
            } else {
                window.set_oauth_profile_loaded(false);
                window.set_oauth_display_name("".into());
                window.set_oauth_username("".into());
                window.set_oauth_user_id("".into());
                window.set_oauth_initials("".into());
                window.set_oauth_bio("".into());
                window.set_oauth_accent_color(slint::Color::from_rgb_u8(35, 36, 40));
            }
            window.set_oauth_has_avatar(images.avatar.is_some());
            window.set_oauth_has_banner(images.banner.is_some());
            window.set_oauth_profile_art_status(images.status.into());
            window.set_oauth_avatar(images.avatar.map(image_from_decoded).unwrap_or_default());
            window.set_oauth_banner(images.banner.map(image_from_decoded).unwrap_or_default());
        }
    }) {
        eprintln!("Could not update Discord profile preview: {error}");
    }
}

fn image_from_decoded(image: DecodedImage) -> slint::Image {
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(image.width, image.height);
    pixels.make_mut_bytes().copy_from_slice(&image.rgba);
    slint::Image::from_rgba8(pixels)
}

fn logout() -> Result<String, String> {
    let session = load_session()?;
    let Some(session) = session else {
        return Ok("Signed out. No saved OAuth2 session was found.".into());
    };

    let token = session
        .refresh_token
        .as_deref()
        .unwrap_or(session.access_token.as_str());
    let revoke_result = post_empty_form(
        REVOKE_URL,
        &[("client_id", session.client_id.as_str()), ("token", token)],
    );
    clear_session()?;
    match revoke_result {
        Ok(()) => Ok("Signed out of Discord; the OAuth2 token was revoked.".into()),
        Err(error) => Ok(format!(
            "Signed out locally, but Discord token revocation failed: {error}"
        )),
    }
}

fn refresh_session(session: &mut Session) -> Result<(), String> {
    let refresh_token = session
        .refresh_token
        .as_deref()
        .ok_or_else(|| "Discord OAuth session has expired; sign in again.".to_string())?;
    let response: TokenResponse = post_form(
        TOKEN_URL,
        &[
            ("client_id", session.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    )?;
    session.access_token.zeroize();
    session.access_token = response.access_token;
    if let Some(refresh_token) = response.refresh_token {
        session.refresh_token = Some(refresh_token);
    }
    session.expires_at = unix_seconds().saturating_add(response.expires_in);
    save_session(session)
}

fn post_empty_form(url: &str, form: &[(&str, &str)]) -> Result<(), String> {
    agent()
        .post(url)
        .send_form(form)
        .map(|_| ())
        .map_err(|error| http_error("Discord token revocation", error))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(15))
        .timeout_write(Duration::from_secs(15))
        .build()
}

fn save_session(session: &Session) -> Result<(), String> {
    let entry = credential_entry()?;
    let serialized = Zeroizing::new(
        serde_json::to_vec(session)
            .map_err(|error| format!("Could not serialize OAuth2 session: {error}"))?,
    );
    entry
        .set_secret(&serialized)
        .map_err(|error| format!("Could not store OAuth2 session in OS credential store: {error}"))
}

fn load_session() -> Result<Option<Session>, String> {
    let entry = credential_entry()?;
    let bytes = Zeroizing::new(match entry.get_secret() {
        Ok(bytes) => bytes,
        Err(KeyringError::NoEntry) => return Ok(None),
        Err(error) => return Err(format!("Could not read OAuth2 session: {error}")),
    });
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("Saved OAuth2 session is invalid: {error}"))
}

fn clear_session() -> Result<(), String> {
    match credential_entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(format!("Could not remove saved OAuth2 session: {error}")),
    }
}

fn credential_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT)
        .map_err(|error| format!("OS credential store is unavailable: {error}"))
}

fn restore_session(ui: &slint::Weak<MainWindow>) {
    match load_session() {
        Ok(Some(mut session)) => {
            let profile = session.profile.clone();
            let images = profile
                .as_ref()
                .map(fetch_profile_images)
                .unwrap_or_else(|| ProfileImages {
                    status: "Sign in again to load Discord profile details.".into(),
                    ..ProfileImages::default()
                });
            set_profile(ui, profile, images);
            let refresh_result = if session.expires_at <= unix_seconds() {
                refresh_session(&mut session)
            } else {
                Ok(())
            };
            match refresh_result {
                Ok(()) => set_ui(
                    ui,
                    true,
                    false,
                    format!(
                        "Signed in to Discord as {}. OAuth is for identity only.",
                        session.username
                    ),
                    Some(true),
                ),
                Err(error) => set_ui(
                    ui,
                    false,
                    false,
                    format!("Saved Discord session could not be refreshed: {error}"),
                    Some(true),
                ),
            }
        }
        Ok(None) => {
            set_profile(ui, None, ProfileImages::default());
            set_ui(ui, false, false, "Not signed in".into(), Some(false));
        }
        Err(error) => set_ui(ui, false, false, error, None),
    }
}

fn random_hex(length: usize) -> Result<String, String> {
    let mut bytes = vec![0; length];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("Could not generate OAuth security value: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn respond(stream: &mut TcpStream, status: u16, message: &str) -> Result<(), String> {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Discord login</title><p>{message}</p>"
    );
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| format!("Could not finish OAuth callback: {error}"))
}

fn http_error(action: &str, error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(status, _) => format!("{action} failed with HTTP {status}."),
        ureq::Error::Transport(error) => format!("{action} failed: {error}"),
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn set_ui(
    ui: &slint::Weak<MainWindow>,
    connected: bool,
    connecting: bool,
    status: String,
    has_session: Option<bool>,
) {
    let ui = ui.clone();
    if let Err(error) = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_oauth_connected(connected);
            window.set_oauth_connecting(connecting);
            window.set_oauth_status(status.into());
            if let Some(has_session) = has_session {
                window.set_oauth_has_session(has_session);
            }
        }
    }) {
        eprintln!("Could not update Discord OAuth2 status: {error}");
    }
}

fn set_authorization_url(ui: &slint::Weak<MainWindow>, authorization_url: String) {
    let ui = ui.clone();
    if let Err(error) = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_oauth_authorize_url(authorization_url.into());
        }
    }) {
        eprintln!("Could not update Discord OAuth2 authorization link: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::{Session, decode_png, parse_callback, parse_relay_callback, profile_initials};

    #[test]
    fn validates_state_issuer_and_authorization_code() {
        assert_eq!(
            parse_callback(
                "/oauth/callback?code=auth-code&state=state-1&iss=https%3A%2F%2Fdiscord.com",
                "state-1"
            )
            .expect("valid callback should succeed"),
            "auth-code"
        );
        assert!(
            parse_callback(
                "/oauth/callback?code=auth-code&state=wrong&iss=https%3A%2F%2Fdiscord.com",
                "state-1"
            )
            .is_err()
        );
        assert!(
            parse_callback(
                "/oauth/callback?code=auth-code&state=state-1&iss=https%3A%2F%2Fevil.example",
                "state-1"
            )
            .is_err()
        );
    }

    #[test]
    fn validates_relayed_kizzy_callback() {
        let relay_path = "/oauth/relay?uri=kizzy%3A%2F%2Foauth2%2Fcallback%3Fcode%3Dauth-code%26state%3Dstate-1%26iss%3Dhttps%253A%252F%252Fdiscord.com";
        assert_eq!(
            parse_relay_callback(relay_path, "state-1")
                .expect("valid Kizzy callback should succeed"),
            "auth-code"
        );
        assert!(
            parse_relay_callback(
                "/oauth/relay?uri=https%3A%2F%2Fevil.example%2Fcallback%3Fcode%3Da",
                "state-1"
            )
            .is_err()
        );
    }

    #[test]
    fn decodes_avatar_png_to_rgba() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("PNG header should encode");
            writer
                .write_image_data(&[12, 34, 56, 255])
                .expect("PNG pixel should encode");
        }

        let decoded = decode_png(&bytes).expect("profile PNG should decode");
        assert_eq!((decoded.width, decoded.height), (1, 1));
        assert_eq!(decoded.rgba, [12, 34, 56, 255]);
    }

    #[test]
    fn derives_profile_fallback_initials() {
        assert_eq!(profile_initials("Music Presence"), "MP");
        assert_eq!(profile_initials("Discord"), "D");
        assert_eq!(profile_initials("   "), "?");
    }

    #[test]
    fn loads_saved_oauth_sessions_without_profile_metadata() {
        let session: Session = serde_json::from_str(
            r#"{
                "client_id": "123456789",
                "access_token": "access",
                "refresh_token": "refresh",
                "expires_at": 2000000000,
                "username": "Discord User"
            }"#,
        )
        .expect("older saved OAuth session should remain readable");
        assert!(session.profile.is_none());
    }
}
