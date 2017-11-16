#[macro_use]
extern crate router;
#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate log;
extern crate pretty_env_logger;

extern crate hyper;
extern crate iron_json_response as ijr;
extern crate iron;
extern crate logger;

extern crate fst;
extern crate fst_levenshtein;
extern crate fst_regex;

extern crate serde;
extern crate rmp_serde as rmps;

extern crate rayon;
extern crate alfred;
extern crate webicon;
extern crate rusqlite;
extern crate url;
extern crate mime;
extern crate glob;

use std::{thread, time};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::iter::FromIterator;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use iron::prelude::*;
use rayon::prelude::*;
use rmps::{Deserializer, Serializer};
use rusqlite::{Connection, Statement};
use serde::{Deserialize, Serialize};
use webicon::IconScraper;


pub mod errors;
pub mod server;
pub mod util;

use server::Server;
use util::*;


const DEFAULT_ICON: &str = "/Applications/Safari.app/Contents/Resources/compass.icns";
const QUERY: &str = "
    SELECT DISTINCT title, url, visit_count_score
    FROM history_visits AS v
    INNER JOIN history_items AS i ON v.history_item = i.id
    ORDER BY visit_count_score DESC, visit_time DESC;";


struct SafariHistory {
    favicons: HashMap<String, String>,
}


impl SafariHistory {
    pub fn new() -> Self {
        let icon_cache_dir = cache_location().join("icons");
        if !icon_cache_dir.exists() {
            debug!("Creating cache dir: {}", &icon_cache_dir.display());
            fs::create_dir_all(&icon_cache_dir).expect("Error on creating cache dir");
        }

        let icon_cache_path = cache_location().join("favicons.cache");
        debug!("Loading favicons from {}", &icon_cache_path.display());

        let mut favicons: HashMap<String, String> = match fs::OpenOptions::new().read(true).open(&icon_cache_path) {
            Ok(mut favicons_cache) => {
                debug!(
                    "Trying to deserialize icon cache from '{}'",
                    &icon_cache_path.display()
                );
                let mut contents = Vec::new();
                favicons_cache.read_to_end(&mut contents).expect(
                    "Error on reading favicons cache",
                );
                let mut de = Deserializer::new(&contents[..]);
                Deserialize::deserialize(&mut de).expect("Can't deserialize cache")
            }
            Err(e) => {
                error!("Error on reading favicons cache file: {}", e);
                HashMap::new()
            }
        };

        debug!("Filling favicon cache from filesystem");
        for direntry in fs::read_dir(icon_cache_dir).expect("Error on reading icon cache dir") {
            match direntry {
                Ok(file) => {
                    if let Some(domain) = file.path().file_stem() {
                        favicons
                            .entry(domain.to_string_lossy().into_owned())
                            .or_insert_with(|| file.path().to_string_lossy().into_owned());
                    }
                }
                Err(e) => {
                    warn!("Couldn't access file: {}", e);
                }
            }

        }

        SafariHistory { favicons }
    }

    fn download_favicon(&self, url: &str) -> String {
        info!("Downloading favicon for {}", &url);

        let mut scraper = IconScraper::from_http(url);
        let icons = scraper.fetch_icons();

        let icon = icons.at_least(64, 64);
        if icon.is_none() {
            return DEFAULT_ICON.to_string();
        }
        let icon = icon.unwrap();

        let contents = icon.raw.clone();
        if contents.is_none() {
            return DEFAULT_ICON.to_string();
        }

        let icon_file = icon_path(icon);
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&icon_file);
        if file.is_err() {
            return DEFAULT_ICON.to_string();
        }

        match file.unwrap().write_all(&contents.unwrap()) {
            Ok(_) => icon_file.to_string_lossy().into_owned(),
            Err(_) => DEFAULT_ICON.to_string(),
        }
    }

    fn get_history_items(&self, stmt: &mut Statement) -> Vec<HistoryItem> {
        stmt.query_map(&[], |row| {
            let score: i64 = row.get(2);
            let url: String = row.get(1);
            let mut title: String = match row.get(0) {
                Some(t) => t,
                None => url.clone(),
            };
            if url.is_empty() || url.starts_with("data:") || url.parse::<hyper::Uri>().is_err() {
                return None;
            }

            if let Ok(parsed_url) = url::Url::parse(&url) {
                if let Some(domain) = parsed_url.host_str() {
                    if title.is_empty() {
                        title = domain.to_string();
                    }
                    Some(HistoryItem {
                        title: title.clone(),
                        url,
                        domain: domain.to_string(),
                        search: format!(
                            "{} | {}{}{}{}",
                            title,
                            domain,
                            parsed_url.path(),
                            parsed_url.query().unwrap_or(""),
                            parsed_url.fragment().unwrap_or("")
                        ).to_lowercase(),
                        score: score as u64,
                        favicon: DEFAULT_ICON.to_string(),
                    })
                } else {
                    None
                }
            } else {
                None
            }
        }).expect("Error on querying history items")
            .filter_map(|r| r.ok())
            .filter_map(|r| r)
            .collect()
    }

    fn run(&mut self, item_sender: &Sender<Vec<HistoryItem>>) {
        let conn = Connection::open(db_location()).expect("Error on connecting to db");
        let mut stmt = conn.prepare(QUERY).expect("Error preparing query");
        loop {
            let mut history_items = self.get_history_items(&mut stmt);
            let new_items: HashSet<HistoryItem> = HashSet::from_iter(
                history_items
                    .iter()
                    .filter(|item| !self.favicons.contains_key(&item.domain))
                    .cloned(),
            );
            let icons: Vec<String> = new_items
                .par_iter()
                .map(|item| self.download_favicon(&item.url))
                .collect();

            for (item, icon) in new_items.iter().zip(icons.iter()) {
                self.favicons.insert(item.domain.clone(), icon.clone());
            }

            for item in &mut history_items {
                item.favicon = self.favicons[&item.domain].clone();
            }

            item_sender.send(history_items).expect(
                "Error on sending Alfred items to server",
            );

            let icon_cache_path = cache_location().join("favicons.cache");
            let mut favicons_cache = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&icon_cache_path)
                .expect("Error on opening favicons.cache");

            debug!(
                "Dumping favicons cache into '{}'",
                &icon_cache_path.display()
            );
            self.favicons
                .serialize(&mut Serializer::new(&mut favicons_cache))
                .expect("Error on caching favicons");

            thread::sleep(time::Duration::from_secs(300));
        }
    }
}

fn main() {
    pretty_env_logger::init().expect("Error on logger init");

    let mut history = SafariHistory::new();
    let server = Server::new();
    let arc_server = Arc::new(server);

    let server1 = Arc::clone(&arc_server);
    let server2 = Arc::clone(&arc_server);
    let server3 = Arc::clone(&arc_server);

    let (item_sender, item_receiver) = channel();

    let router = router!(search: get "/search/:search_type/:query" => move |req: &mut Request| server1.search(req));

    thread::spawn(move || server2.update_items(&item_receiver));
    thread::spawn(move || server3.run(6020, router));

    history.run(&item_sender);
}