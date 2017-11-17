use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Receiver;

use fst::{IntoStreamer, Map};
use fst::automaton::Automaton;
use fst_levenshtein::Levenshtein;
use fst_regex::Regex;
use ijr::{JsonResponse, JsonResponseMiddleware};
use iron::prelude::*;
use logger::Logger;
use router::Router;

use errors::*;
use util::cache_location;

const SEARCH_TYPE_REGEX: &str = "regex";
const GOOGLE_SEARCH_URL: &str = "https://google.com/search";
const GOOGLE_FEELING_LUCKY_URL: &str = "https://www.google.com/webhp?#btnI=I";
const SEARCH_WITH_GOOGLE: &str = "Search with Google";
const IM_FEELING_LUCKY: &str = "I'm feeling lucky!";


pub struct Server<'a> {
    item_index: Arc<Mutex<Map>>,
    items: Arc<Mutex<BTreeMap<String, ::alfred::Item<'a>>>>,
    lev_distance: Arc<Mutex<u32>>,
}

impl<'a> Server<'a> {
    pub fn new() -> Self {
        Self {
            items: Arc::new(Mutex::new(BTreeMap::new())),
            item_index: Arc::new(Mutex::new(Map::from_iter(vec![("a", 0)]).expect(
                "Error on creating empty Map",
            ))),
            lev_distance: Arc::new(Mutex::new(0)),
        }
    }

    fn get_items<A: Automaton>(&self, query: A) -> Result<Vec<::alfred::Item>> {
        let items = self.items.lock().expect("Error on locking Alfred items");
        let item_index = self.item_index.lock().expect("Error on locking index");

        let mut keys = item_index.search(query).into_stream().into_str_vec()?;
        keys.sort_unstable_by_key(|&(_, score)| score);

        Ok(
            keys.iter()
                .filter_map(|&(ref key, _)| items.get(key))
                .rev()
                .take(20)
                .cloned()
                .collect(),
        )
    }

    pub fn search(&self, req: &mut Request) -> IronResult<Response> {
        let router = req.extensions.get::<Router>().expect(
            "Error on getting router extension",
        );
        let query = router.find("query").unwrap_or("*");
        let search_type = router.find("search_type").unwrap_or("fuzzy");

        debug!("{} searching for {}...", search_type, query);
        let mut items = match search_type {
            SEARCH_TYPE_REGEX => {

                let re = Regex::new(&format!(".*{}.*", query.replace("%20", r".*")))
                    .map_err(Error::from)?;
                self.get_items(re)?
            }
            _ => {
                let lev = Levenshtein::new(query, 10).map_err(Error::from)?;
                self.get_items(lev)?
            }
        };
        if items.is_empty() {
            let google_url = format!("{}?q={}", GOOGLE_SEARCH_URL, query);
            items = vec![
                ::alfred::ItemBuilder::new(query.clone())
                    .text_copy(query.clone())
                    .text_large_type(query.clone())
                    .quicklook_url(google_url.clone())
                    .arg(google_url)
                    .arg_mod(::alfred::Modifier::Option, format!("{}&q={}", GOOGLE_FEELING_LUCKY_URL, query))
                    .subtitle_mod(::alfred::Modifier::Option, IM_FEELING_LUCKY)
                    .icon_path_mod(::alfred::Modifier::Option, ::DEFAULT_ICON.to_string())
                    .subtitle(SEARCH_WITH_GOOGLE)
                    .icon_path(cache_location().join("icons").join("google.com.ico").to_string_lossy().into_owned())
                    .into_item()
            ];
        }
        let data = ::alfred::json::Builder::with_items(&items).into_json();
        Ok(Response::with(
            (::iron::status::Ok, JsonResponse::json(data)),
        ))
    }

    pub fn update_items(&self, item_receiver: &Receiver<Vec<::HistoryItem>>) {
        while let Ok(history_items) = item_receiver.recv() {
            let mut item_index = self.item_index.lock().expect("Error on locking index");
            let mut items = self.items.lock().expect("Error on locking Alfred items");
            let mut urls: BTreeMap<String, u64> = BTreeMap::new();
            let mut lev_distance = self.lev_distance.lock().expect(
                "Error on locking lev distance",
            );

            debug!("Updating Alfred items");
            for item in &history_items {
                let alfred_item = self.get_alfred_item(item.clone());
                *lev_distance = lev_distance.max(item.search.len() as u32);
                urls.insert(item.search.clone(), item.score);
                if !items.contains_key(&item.search) {
                    items.insert(item.search.clone(), alfred_item);
                }
            }

            let mut urls: Vec<(String, u64)> = urls.into_iter().collect();
            urls.sort_unstable_by_key(|&(ref key, _)| key.clone());

            debug!("Indexing Alfred items");
            *item_index = Map::from_iter(urls).expect("Error on indexing Alfred items");
            debug!("Finished indexing Alfred items");
        }
    }

    fn get_alfred_item(&self, item: ::HistoryItem) -> ::alfred::Item<'a> {
        ::alfred::ItemBuilder::new(item.title.clone())
            .autocomplete(item.title.clone())
            .uid(item.url.clone())
            .text_copy(item.url.clone())
            .text_large_type(item.url.clone())
            .quicklook_url(item.url.clone())
            .arg(item.url.clone())
            // .arg_mod(::alfred::Modifier::Command, format!("{}?q={}", GOOGLE_SEARCH_URL, item.title))
            // .subtitle_mod(::alfred::Modifier::Command, SEARCH_WITH_GOOGLE)
            // .icon_path_mod(::alfred::Modifier::Command, cache_location().join("icons").join("google.com.ico").to_string_lossy().into_owned())
            // .arg_mod(::alfred::Modifier::Option, format!("{}&q={}", GOOGLE_FEELING_LUCKY_URL, item.title))
            // .subtitle_mod(::alfred::Modifier::Option, IM_FEELING_LUCKY)
            // .icon_path_mod(::alfred::Modifier::Option, ::DEFAULT_ICON.to_string())
            .subtitle(item.url)
            .icon_path(item.favicon)
            .variable("score", item.score.to_string())
            .into_item()
    }

    pub fn run(&self, port: u64, router: Router) {
        let (logger_before, logger_after) = Logger::new(None);

        let mut chain = Chain::new(router);
        chain.link_before(logger_before);
        chain.link_after(JsonResponseMiddleware::new());
        chain.link_after(logger_after);

        Iron::new(chain)
            .http(format!("localhost:{}", port))
            .expect("Couldn't start server");
    }
}