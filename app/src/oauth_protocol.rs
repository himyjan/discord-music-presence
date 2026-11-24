use std::{
    env,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    process::Command,
    time::Duration,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::{fs, path::Path, path::PathBuf};

use url::Url;

const SCHEME: &str = "kizzy";
const CALLBACK_HOST: &str = "oauth2";
const CALLBACK_PATH: &str = "/callback";
const CALLBACK_PORT: u16 = 49153;
#[cfg(target_os = "macos")]
const MACOS_HANDLER_BUNDLE_ID: &str = "com.musicpresence.oauth-handler";

pub fn callback_argument() -> Option<String> {
    env::args_os().skip(1).find_map(|argument| {
        let argument = argument.into_string().ok()?;
        let prefix = format!("{SCHEME}://");
        argument.starts_with(&prefix).then_some(argument)
    })
}

pub fn forward_callback(uri: &str) -> Result<(), String> {
    validate_callback_uri(uri)?;

    let mut relay_url = Url::parse(&format!("http://127.0.0.1:{CALLBACK_PORT}/oauth/relay"))
        .map_err(|error| format!("Could not construct OAuth callback relay URL: {error}"))?;
    relay_url.query_pairs_mut().append_pair("uri", uri);

    let address = SocketAddr::from(([127, 0, 0, 1], CALLBACK_PORT));
    let mut stream =
        TcpStream::connect_timeout(&address, Duration::from_secs(3)).map_err(|error| {
            format!("Could not relay the Discord callback to the running app: {error}")
        })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("Could not set OAuth relay timeout: {error}"))?;

    let request_target = match relay_url.query() {
        Some(query) if !query.is_empty() => format!("{}?{query}", relay_url.path()),
        _ => relay_url.path().to_owned(),
    };
    write!(
        stream,
        "GET {request_target} HTTP/1.1\r\nHost: 127.0.0.1:{CALLBACK_PORT}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|error| format!("Could not send the Discord callback to the running app: {error}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("Could not read the OAuth relay response: {error}"))?;
    let status_line = response.lines().next().unwrap_or_default();
    if status_line.contains(" 200 ") {
        Ok(())
    } else {
        Err(format!(
            "The running app rejected the OAuth callback: {status_line}"
        ))
    }
}

pub fn register_handler() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        register_macos()
    }
    #[cfg(target_os = "windows")]
    {
        register_windows()
    }
    #[cfg(target_os = "linux")]
    {
        register_linux()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err("OAuth callback protocol registration is unsupported on this platform.".into())
    }
}

fn validate_callback_uri(uri: &str) -> Result<Url, String> {
    let url = Url::parse(uri)
        .map_err(|_| "Discord returned an invalid OAuth callback URL.".to_string())?;
    if url.scheme() != SCHEME
        || url.host_str() != Some(CALLBACK_HOST)
        || url.path() != CALLBACK_PATH
        || url.query().is_none()
    {
        return Err("The OAuth callback URL has an unexpected scheme or path.".into());
    }
    Ok(url)
}

#[cfg(target_os = "windows")]
fn register_windows() -> Result<(), String> {
    let executable = env::current_exe()
        .map_err(|error| format!("Could not locate the app executable: {error}"))?;
    let command = format!("\"{}\" \"%1\"", executable.display());
    let key = format!(r"HKCU\Software\Classes\{SCHEME}");
    let command_key = format!(r"HKCU\Software\Classes\{SCHEME}\shell\open\command");

    run_command(
        Command::new("reg.exe").args(["add", &key, "/ve", "/d", "URL:Kizzy OAuth Protocol", "/f"]),
        "Could not register the Kizzy OAuth URL protocol",
    )?;
    run_command(
        Command::new("reg.exe").args(["add", &key, "/v", "URL Protocol", "/d", "", "/f"]),
        "Could not mark the Kizzy OAuth URL protocol",
    )?;
    run_command(
        Command::new("reg.exe").args(["add", &command_key, "/ve", "/d", &command, "/f"]),
        "Could not register the Kizzy OAuth callback command",
    )
}

#[cfg(target_os = "linux")]
fn register_linux() -> Result<(), String> {
    let executable = env::current_exe()
        .map_err(|error| format!("Could not locate the app executable: {error}"))?;
    let executable = executable
        .to_str()
        .ok_or_else(|| "The app executable path is not valid UTF-8.".to_string())?;
    let exec_argument = desktop_exec_argument(executable);
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .ok_or_else(|| "Could not determine the per-user applications directory.".to_string())?;
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications)
        .map_err(|error| format!("Could not create the user applications directory: {error}"))?;
    let desktop_file = applications.join("music-presence-oauth.desktop");
    fs::write(
        &desktop_file,
        format!(
            "[Desktop Entry]\nType=Application\nName=Music Presence OAuth Callback\nNoDisplay=true\nTerminal=false\nExec={exec_argument} %u\nMimeType=x-scheme-handler/{SCHEME};\n"
        ),
    )
    .map_err(|error| format!("Could not write the OAuth desktop handler: {error}"))?;

    let mime_type = format!("x-scheme-handler/{SCHEME}");
    run_command(
        Command::new("xdg-mime").args(["default", "music-presence-oauth.desktop", &mime_type]),
        "Could not set the Kizzy OAuth URL handler",
    )
}

#[cfg(target_os = "linux")]
fn desktop_exec_argument(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '\\' | '"' | '$' | '`' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '%' => escaped.push_str("%%"),
            _ => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(target_os = "macos")]
fn register_macos() -> Result<(), String> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "Could not locate the current user's home directory.".to_string())?;
    let support_dir = home.join("Library/Application Support/Music Presence");
    fs::create_dir_all(&support_dir)
        .map_err(|error| format!("Could not create the OAuth handler directory: {error}"))?;
    let app_bundle = support_dir.join("Kizzy OAuth Callback.app");
    let info_plist = app_bundle.join("Contents/Info.plist");

    let needs_build = if app_bundle.exists() {
        let owned_by_app = info_plist.exists() && {
            let output = Command::new("/usr/libexec/PlistBuddy")
                .args(["-c", "Print :CFBundleIdentifier"])
                .arg(&info_plist)
                .output()
                .map_err(|error| format!("Could not inspect the existing OAuth handler: {error}"))?;
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).trim() == MACOS_HANDLER_BUNDLE_ID
        };

        if !owned_by_app {
            fs::remove_dir_all(&app_bundle).map_err(|error| {
                format!(
                    "Could not replace existing unowned OAuth handler at {}: {error}",
                    app_bundle.display()
                )
            })?;
            true
        } else {
            false
        }
    } else {
        true
    };

    if needs_build {
        compile_macos_handler(&support_dir, &app_bundle, &info_plist)?;
    }

    let register = Command::new(
        "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
    )
    .args(["-f"])
    .arg(&app_bundle)
    .output()
    .map_err(|error| format!("Could not register the macOS OAuth handler: {error}"))?;
    if !register.status.success() {
        return Err(format!(
            "Could not register the macOS OAuth handler: {}",
            String::from_utf8_lossy(&register.stderr).trim()
        ));
    }
    set_macos_default_handler()
}

#[cfg(target_os = "macos")]
fn compile_macos_handler(
    support_dir: &Path,
    app_bundle: &Path,
    info_plist: &Path,
) -> Result<(), String> {
    let applescript = format!(
        r#"on open location callbackURL
    set relayQuery to "uri=" & callbackURL
    do shell script "/usr/bin/curl --fail --silent --show-error --max-time 8 --get --data-urlencode " & quoted form of relayQuery & " http://127.0.0.1:{CALLBACK_PORT}/oauth/relay"
end open location

on run
    return
end run
"#
    );

    let source = support_dir.join("kizzy-oauth-callback.applescript");
    fs::write(&source, applescript)
        .map_err(|error| format!("Could not write the OAuth callback handler: {error}"))?;
    let compile_result = Command::new("/usr/bin/osacompile")
        .args(["-o"])
        .arg(app_bundle)
        .arg(&source)
        .output()
        .map_err(|error| format!("Could not compile the macOS OAuth handler: {error}"));
    let remove_result = fs::remove_file(&source);
    let output = compile_result?;
    remove_result
        .map_err(|error| format!("Could not remove the temporary OAuth script: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not compile the macOS OAuth handler: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    configure_macos_handler(info_plist)
}

#[cfg(target_os = "macos")]
fn configure_macos_handler(info_plist: &Path) -> Result<(), String> {
    let run_plist = |cmd: &str| -> Result<(), String> {
        let output = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", cmd])
            .arg(info_plist)
            .output()
            .map_err(|error| format!("Could not configure the macOS OAuth handler: {error}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Could not configure the macOS OAuth handler ({cmd}): {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    };

    run_plist(&format!("Set :CFBundleIdentifier {MACOS_HANDLER_BUNDLE_ID}"))?;

    // Remove any pre-existing CFBundleURLTypes so re-configuring never fails on duplicate keys.
    let _ = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Delete :CFBundleURLTypes"])
        .arg(info_plist)
        .output();

    run_plist("Add :CFBundleURLTypes array")?;
    run_plist("Add :CFBundleURLTypes:0 dict")?;
    run_plist("Add :CFBundleURLTypes:0:CFBundleURLName string Music Presence OAuth Callback")?;
    run_plist("Add :CFBundleURLTypes:0:CFBundleURLSchemes array")?;
    run_plist(&format!("Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string {SCHEME}"))?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn set_macos_default_handler() -> Result<(), String> {
    use std::ffi::{CString, c_char, c_void};

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            string: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFRelease(value: *const c_void);
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSSetDefaultHandlerForURLScheme(
            scheme: *const c_void,
            handler_bundle_id: *const c_void,
        ) -> i32;
    }

    let scheme = CString::new(SCHEME).map_err(|error| error.to_string())?;
    let bundle_id = CString::new(MACOS_HANDLER_BUNDLE_ID).map_err(|error| error.to_string())?;
    let (scheme_string, bundle_string) = unsafe {
        (
            CFStringCreateWithCString(std::ptr::null(), scheme.as_ptr(), 0x0800_0100),
            CFStringCreateWithCString(std::ptr::null(), bundle_id.as_ptr(), 0x0800_0100),
        )
    };
    if scheme_string.is_null() || bundle_string.is_null() {
        if !scheme_string.is_null() {
            unsafe { CFRelease(scheme_string) };
        }
        if !bundle_string.is_null() {
            unsafe { CFRelease(bundle_string) };
        }
        return Err("Could not create the macOS OAuth handler identifiers.".into());
    }

    let status = unsafe { LSSetDefaultHandlerForURLScheme(scheme_string, bundle_string) };
    unsafe {
        CFRelease(scheme_string);
        CFRelease(bundle_string);
    }
    if status == 0 {
        Ok(())
    } else {
        Err(format!(
            "Could not set the default macOS OAuth handler (Launch Services status {status})."
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn run_command(command: &mut Command, action: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("{action}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{action}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::validate_callback_uri;

    #[cfg(target_os = "linux")]
    use super::desktop_exec_argument;

    #[test]
    fn accepts_only_the_kizzy_oauth_callback() {
        assert!(
            validate_callback_uri("kizzy://oauth2/callback?code=auth-code&state=state-1").is_ok()
        );
        assert!(validate_callback_uri("https://example.com/callback?code=auth-code").is_err());
        assert!(validate_callback_uri("kizzy://other/callback?code=auth-code").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn quotes_linux_handler_paths_for_desktop_entries() {
        assert_eq!(
            desktop_exec_argument("/home/user/Music Presence/music-presence"),
            "\"/home/user/Music Presence/music-presence\""
        );
        assert_eq!(
            desktop_exec_argument("/home/user/50% complete/music-presence"),
            "\"/home/user/50%% complete/music-presence\""
        );
    }
}