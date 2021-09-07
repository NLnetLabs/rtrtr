//! A target using the HTTP server.

use std::convert::Infallible;
use std::sync::Arc;
use arc_swap::ArcSwap;
use futures::stream;
use hyper::{Body, Method, Request, Response};
use log::debug;
use serde::Deserialize;
use crate::payload;
use crate::comms::Link;
use crate::formats::output;
use crate::log::ExitError;
use crate::manager::Component;


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
            move |request: &Request<_>| {
                if 
                    request.method() != Method::GET
                    || request.uri().path() != path
                {
                    return None
                }

                if let Some(update) = http_source.set() {
                    Some(
                        Response::builder()
                        .header("Content-Type", format.content_type())
                        .body(Body::wrap_stream(stream::iter(
                            format.stream(update)
                            .map(Result::<_, Infallible>::Ok)
                        )))
                        .unwrap()
                    )
                }
                else {
                    Some(
                        Response::builder()
                        .status(503)
                        .header("Content-Type", "text/plain")
                        .body("Initial validation ongoing. Please wait.".into())
                        .unwrap()
                    )
                }
            }
        );
        component.register_http_resource(processor.clone());

        loop {
            debug!("Target {}: link status: {}",
                    component.name(), unit.get_status()
            );
            if let Ok(update) = unit.query().await {
                debug!(
                    "Target {}: Got update ({} entries)",
                    component.name(), update.set().len()
                );
                source.update(update);
            }
        }
    }
}



//------------ Source --------------------------------------------------------

#[derive(Clone, Default)]
struct Source {
    /// The current set of RTR data.
    data: Arc<ArcSwap<Option<payload::Set>>>
}

impl Source {
    fn update(&self, update: payload::Update) {
        self.data.store(Some(update.set().clone()).into())
    }

    fn set(&self) -> Option<payload::Set> {
        match self.data.load().as_ref() {
            Some(ref inner) => Some(inner.clone()),
            None => None
        }
    }
}


