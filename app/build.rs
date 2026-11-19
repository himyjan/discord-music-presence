use roxmltree::{Document, Node, NodeType};
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Serialize)]
struct Language {
    code: String,
    label: String,
}

#[derive(Serialize)]
struct LocaleBundle {
    languages: Vec<Language>,
    translations: BTreeMap<String, BTreeMap<String, String>>,
}

fn read_strings(path: &Path) -> BTreeMap<String, String> {
    println!("cargo:rerun-if-changed={}", path.display());
    let xml = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("Could not read {}: {error}", path.display()));
    let document = Document::parse(&xml)
        .unwrap_or_else(|error| panic!("Could not parse {}: {error}", path.display()));

    document
        .descendants()
        .filter(|node| node.has_tag_name("string"))
        .filter_map(|node| {
            let name = node.attribute("name")?;
            let mut value = String::new();
            collect_text(node, &mut value);
            Some((name.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn collect_text(node: Node<'_, '_>, value: &mut String) {
    for child in node.children() {
        match child.node_type() {
            NodeType::Text => value.push_str(child.text().unwrap_or_default()),
            NodeType::Element => collect_text(child, value),
            _ => {}
        }
    }
}

fn resource_directory(code: &str) -> String {
    match code {
        "en" => "values".to_owned(),
        "zh-hans" => "values-zh-rCN".to_owned(),
        "zh-hant" => "values-zh-rTW".to_owned(),
        _ => format!("values-{code}"),
    }
}

fn main() {
    let resources = PathBuf::from("src/main/res");
    let default_resources = resources.join("values");
    let english = read_strings(&default_resources.join("strings.xml"));
    let language_names = read_strings(&default_resources.join("translations.xml"));
    let mut languages = Vec::new();
    let mut translations = BTreeMap::new();

    for (name, label) in language_names {
        let Some(code) = name.strip_prefix("locale_") else {
            continue;
        };
        let code = code.replace('_', "-");
        let strings_path = resources
            .join(resource_directory(&code))
            .join("strings.xml");
        let mut localized = BTreeMap::new();

        if strings_path.is_file() {
            for (key, translated) in read_strings(&strings_path) {
                if let Some(source) = english.get(&key) {
                    if translated != *source {
                        localized.entry(source.clone()).or_insert(translated);
                    }
                }
            }
        }

        languages.push(Language {
            code: code.clone(),
            label,
        });
        translations.insert(code, localized);
    }

    if languages.len() != 88 {
        panic!(
            "Expected 88 locale labels in values/translations.xml, found {}",
            languages.len()
        );
    }

    let mut label_counts = HashMap::new();
    for language in &languages {
        *label_counts.entry(language.label.clone()).or_insert(0usize) += 1;
    }
    for language in &mut languages {
        if label_counts[&language.label] > 1 {
            language.label = format!("{} ({})", language.label, language.code);
        }
    }
    languages.sort_by_key(|language| language.label.to_lowercase());

    let bundle = LocaleBundle {
        languages,
        translations,
    };
    let serialized = serde_json::to_string(&bundle)
        .expect("Could not serialize the embedded localization bundle");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set"));
    fs::write(
        output.join("localization_bundle.rs"),
        format!("pub const LOCALIZATION_BUNDLE: &str = {serialized:?};"),
    )
    .expect("Could not write the embedded localization bundle");

    println!("cargo:rerun-if-changed={}", resources.display());
    slint_build::compile("ui/main.slint").expect("failed to compile Slint UI");
}
