use error_chain::ChainedError;
use ijr::JsonResponse;
use iron::error::IronError;

error_chain! {
    foreign_links {
        Fst(::fst::Error);
        Regex(::fst_regex::Error);
        Levenshtein(::fst_levenshtein::Error);
        ParseURL(::url::ParseError);
        WriteIcon(::std::io::Error);
        Glob(::glob::GlobError);
        ParsePattern(::glob::PatternError);
    }

    errors {
        EmptyIcon(domain: String) {
            description("An empty icon was downloaded")
            display("An empty icon was downloaded: '{}'", domain)
        }
        NoIcon(domain: String) {
            description("No icon was found")
            display("No icon was found: '{}'", domain)
        }
        NoMimetype(domain: String) {
            description("The icon has no detected mimetype")
            display("The icon has no detected mimetype: '{}'", domain)
        }
    }
}

impl From<Error> for IronError {
    fn from(e: Error) -> IronError {
        let data = ::alfred::json::Builder::with_items(
            &[
                ::alfred::ItemBuilder::new(e.description())
                    .autocomplete(e.to_string())
                    .text_copy(e.display_chain().to_string())
                    .text_large_type(e.display_chain().to_string())
                    .subtitle(e.to_string())
                    .into_item(),
            ],
        ).into_json();
        IronError::new(e, JsonResponse::json(data))
    }
}