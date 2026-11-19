use serde::Deserialize;
use std::{
    collections::HashMap,
    env, fs,
    io::{self, ErrorKind},
    path::PathBuf,
};

include!(concat!(env!("OUT_DIR"), "/localization_bundle.rs"));

#[derive(Clone, Debug, Deserialize)]
pub struct Language {
    pub code: String,
    pub label: String,
}

#[derive(Deserialize)]
struct Bundle {
    languages: Vec<Language>,
    translations: HashMap<String, HashMap<String, String>>,
}

pub struct Localizer {
    languages: Vec<Language>,
    translations: HashMap<String, HashMap<String, String>>,
}

impl Localizer {
    pub fn bundled() -> Self {
        let bundle: Bundle = serde_json::from_str(LOCALIZATION_BUNDLE)
            .expect("the generated localization bundle must be valid JSON");
        Self {
            languages: bundle.languages,
            translations: bundle.translations,
        }
    }

    pub fn languages(&self) -> &[Language] {
        &self.languages
    }

    pub fn label_for_code(&self, code: &str) -> Option<&str> {
        self.languages
            .iter()
            .find(|language| language.code == code)
            .map(|language| language.label.as_str())
    }

    pub fn code_for_label(&self, label: &str) -> Option<&str> {
        self.languages
            .iter()
            .find(|language| language.label == label)
            .map(|language| language.code.as_str())
    }

    pub fn translate(&self, code: &str, source: &str) -> String {
        self.translations
            .get(code)
            .and_then(|strings| strings.get(source))
            .cloned()
            .unwrap_or_else(|| source.to_owned())
    }

    pub fn load_saved_language(&self) -> String {
        let path = match preference_path() {
            Ok(path) => path,
            Err(error) => {
                eprintln!("Could not locate the language preference file: {error}");
                return "en".to_owned();
            }
        };

        match fs::read_to_string(path) {
            Ok(code) if self.label_for_code(code.trim()).is_some() => code.trim().to_owned(),
            Ok(code) => {
                eprintln!("Ignoring unsupported saved language code: {}", code.trim());
                "en".to_owned()
            }
            Err(error) if error.kind() == ErrorKind::NotFound => "en".to_owned(),
            Err(error) => {
                eprintln!("Could not read the saved language preference: {error}");
                "en".to_owned()
            }
        }
    }

    pub fn save_language(&self, code: &str) -> io::Result<()> {
        if self.label_for_code(code).is_none() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("unsupported language code: {code}"),
            ));
        }

        let path = preference_path()?;
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidInput,
                "language preference path has no parent",
            )
        })?;
        fs::create_dir_all(parent)?;
        fs::write(path, code)
    }
}

fn preference_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let directory = env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "APPDATA is not set"))?
        .join("Music Presence");

    #[cfg(target_os = "macos")]
    let directory = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "HOME is not set"))?
        .join("Library/Application Support/Music Presence");

    #[cfg(all(unix, not(target_os = "macos")))]
    let directory = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                "neither XDG_CONFIG_HOME nor HOME is set",
            )
        })?
        .join("music-presence");

    Ok(directory.join("language"))
}

#[cfg(test)]
mod tests {
    use super::Localizer;

    #[test]
    fn embeds_eighty_eight_language_choices() {
        assert_eq!(Localizer::bundled().languages().len(), 88);
    }

    #[test]
    fn translates_available_strings_and_falls_back_to_english() {
        let localizer = Localizer::bundled();
        assert_ne!(localizer.translate("es", "Language"), "Language");
        assert_eq!(
            localizer.translate("es", "An app-specific string without a translation"),
            "An app-specific string without a translation"
        );
    }
}
