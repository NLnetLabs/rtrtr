//! All supported output formats.


use std::sync::Arc;
use serde::Deserialize;
use crate::payload;
use super::json;

//------------ Format --------------------------------------------------------


/// The output format for VRPs.
#[derive(Clone, Copy, Debug, Deserialize)]
pub enum Format {
    #[serde(rename = "json")]
    Json,
}

impl Format {
    pub fn content_type(self) -> &'static str {
        match self {
            Format::Json => "application/json",
        }
    }

    pub fn stream(self, set: Arc<payload::Set>) -> Stream {
        Stream::new(self, set)
    }
}


//------------ Stream --------------------------------------------------------

pub struct Stream(StreamInner);

enum StreamInner {
    Json(json::OutputStream),
}

impl Stream {
    fn new(format: Format, set: Arc<payload::Set>) -> Self {
        Stream(match format {
            Format::Json => StreamInner::Json(json::OutputStream::new(set)),
        })
    }
}

impl Iterator for Stream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.0 {
            StreamInner::Json(ref mut inner) => inner.next()
        }
    }
}

