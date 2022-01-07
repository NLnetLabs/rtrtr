//! All supported output formats.


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

    pub fn stream(self, set: payload::Set) -> Stream {
        Stream::new(self, set)
    }
}


//------------ Stream --------------------------------------------------------

/// A stream of formatted output.
pub struct Stream(StreamInner);

enum StreamInner {
    Json(json::OutputStream),
}

impl Stream {
    /// Creates a new output stream from a format and a data set.
    fn new(format: Format, set: payload::Set) -> Self {
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

