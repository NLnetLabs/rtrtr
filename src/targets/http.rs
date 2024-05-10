//! A target using the HTTP server.

use std::sync::Arc;
use arc_swap::ArcSwap;
use daemonbase::error::ExitError;
use chrono::{DateTime, Utc};
use futures_util::stream;
use hyper::Method;
use hyper::header::{IF_NONE_MATCH, IF_MODIFIED_SINCE};
use log::debug;
use rpki::rtr::State;
use serde::Deserialize;
use crate::payload;
use crate::comms::{Link, UnitUpdate};
use crate::formats::output;
use crate::http::{ContentType, Response, ResponseBuilder, Request};
use crate::manager::Component;
use crate::utils::http::EtagsIter;
use crate::utils::http::parse_http_date;


//------------ Target --------------------------------------------------------

/// A target using the HTTP server.
#[derive(Debug, Deserialize)]
pub struct Target {
    path: String,
    format: output::Format,
    unit: Link,
}

impl Target {
    /// Runs the target.
    pub async fn run(
        self, mut component: Component
    ) -> Result<(), ExitError> {
        let source = Source::default();
        let (path, format, mut unit) = (self.path, self.format, self.unit);

        let http_source = source.clone();
        
        let processor = Arc::new(
            move |request: &Request| {
                if 
                    request.method() != Method::GET
                    || request.uri().path() != path
                {
                    return None
                }

                let update = http_source.data();
                let update = match update.as_ref() {
                    Some(update) => update,
                    None => {
                        return Some(
                            ResponseBuilder::service_unavailable()
                            .content_type(ContentType::TEXT)
                            .body("Initial validation ongoing. Please wait.")
                        )
                    }
                };

                if update.is_not_modified(request) {
                    return Some(update.not_modified())
                }

                Some(
                    ResponseBuilder::ok()
                    .content_type(format.content_type())
                    .etag(&update.etag)
                    .last_modified(update.created)
                    .stream(
                        stream::iter(
                            format.stream(update.set.clone()).map(Into::into)
                        )
                    )
                )
            }
        );
        component.register_http_resource(processor.clone());

        let mut state = State::new();

        loop {
            debug!("Target {}: link status: {}",
                    component.name(), unit.health()
            );
            if let UnitUpdate::Payload(update) = unit.query().await {
                debug!(
                    "Target {}: Got update ({} entries)",
                    component.name(), update.set().len()
                );
                source.update(SourceData::new(&update, &mut state));
            }
        }
    }
}


//------------ Source --------------------------------------------------------

/// The date source for an HTTP target.
#[derive(Clone, Default)]
struct Source {
    /// The current set of RTR data.
    data: Arc<ArcSwap<Option<SourceData>>>
}

impl Source {
    /// Updates the data source from the given update.
    fn update(&self, data: SourceData) {
        self.data.store(Some(data).into())
    }

    /// Returns the current payload data.
    fn data(&self) -> Arc<Option<SourceData>> {
        self.data.load_full()
    }
}


//------------ SourceData ----------------------------------------------------

/// The data held by a data source.
///
struct SourceData {
    set: payload::Set,
    etag: String,
    created: DateTime<Utc>,
}

impl SourceData {
    fn new(update: &payload::Update, state: &mut State) -> Self {
        let etag = format!("\"{:x}-{}\"", state.session(), state.serial());
        state.inc();
        Self {
            set: update.set().clone(),
            etag,
            created: Utc::now(),
        }
    }

    /// Returns whether 304 Not Modified response should be returned.
    fn is_not_modified(&self, req: &Request) -> bool {
        // First, check If-None-Match.
        let mut found_if_none_match = false;
        for value in req.headers().get_all(IF_NONE_MATCH).iter() {
            found_if_none_match = true;

            // Skip ill-formatted values. By being lazy here we may falsely
            // return a full response, so this should be fine.
            let value = match value.to_str() {
                Ok(value) => value,
                Err(_) => continue
            };
            let value = value.trim();
            if value == "*" {
                return true
            }
            for tag in EtagsIter::new(value) {
                if tag.trim() == self.etag {
                    return true
                }
            }
        }

        // If there was at least one If-None-Match, we are supposed to
        // ignore If-Modified-Since.
        if found_if_none_match {
            return false
        }

        // Check the If-Modified-Since header.
        if let Some(value) = req.headers().get(IF_MODIFIED_SINCE) {
            let value = match value.to_str() {
                Ok(value) => value,
                Err(_) => return false,
            };
            if let Some(date) = parse_http_date(value) {
                if date >= self.created {
                    return true
                }
            }
        }

        false
    }

    fn not_modified(&self) -> Response {
        ResponseBuilder::not_modified()
            .etag(&self.etag)
            .last_modified(self.created)
            .empty()
    }
}
