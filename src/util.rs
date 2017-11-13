use std::env;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use mime;
use url::Url;
use webicon::Icon;


const CACHE_DIR: &str = ".cache/safari_history";
const DB_LOCATION: &str = "Library/Safari/History.db";

pub fn cache_location() -> PathBuf {
    env::home_dir().expect("Error on getting home dir").join(
        CACHE_DIR,
    )
}

pub fn icon_path(icon: Icon) -> PathBuf {
    let domain = icon.url.host().unwrap();
    let mimetype = icon.mime_type.unwrap_or(mime::IMAGE_PNG);
    let mut extension = mimetype.subtype().as_str();
    if extension == "x-icon" {
        extension = "ico";
    }

    cache_location().join("icons").join(format!(
        "{}.{}",
        domain,
        extension
    ))
}

pub fn db_location() -> PathBuf {
    env::home_dir().expect("Error on getting home dir").join(
        DB_LOCATION,
    )
}

pub fn get_domain(url: &str) -> Option<String> {
    if let Ok(parsed_url) = Url::parse(url) {
        if let Some(domain) = parsed_url.host() {
            return Some(format!("{}", &domain));
        }
    }
    None
}

#[derive(Clone)]
pub struct HistoryItem {
    pub title: String,
    pub url: String,
    pub domain: String,
    pub search: String,
    pub score: u64,
    pub favicon: String,
}

impl PartialEq for HistoryItem {
    fn eq(&self, other: &HistoryItem) -> bool {
        self.domain == other.domain
    }
}

impl Eq for HistoryItem {}

impl Hash for HistoryItem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.domain.hash(state);
    }
}