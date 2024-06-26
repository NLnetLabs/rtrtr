//! JSON clients.

use std::{cmp, io};
use std::str::FromStr;
use std::time::{Duration, SystemTime};
use std::fs::metadata;
use chrono::{DateTime, Utc};
use bytes::{Buf, Bytes, BytesMut};
use daemonbase::config::ConfigPath;
use daemonbase::error::Failed;
use log::{debug, warn};
use reqwest::header;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::task::spawn_blocking;
use tokio::time::{Instant, timeout_at};
use crate::payload;
use crate::comms::{Gate, Terminated, UnitUpdate};
use crate::formats::json::Set as JsonSet;
use crate::manager::Component;
use crate::utils::http::{format_http_date, parse_http_date};


//------------ Json ----------------------------------------------------------

/// An unit that regularly fetches a JSON-encoded VRP set.
#[derive(Debug, Deserialize)]
pub struct Json {
    /// The URI of the JSON source.
    uri: SourceUri,

    /// How many seconds to wait before refreshing the data.
    refresh: u64,
}

impl Json {
    pub async fn run(
        mut self, mut component: Component, mut gate: Gate
    ) -> Result<(), Terminated> {
        component.register_metrics(gate.metrics());
        loop {
            self.step(&component, &mut gate).await?;
            self.wait(&mut gate).await?;
        }
    }

    async fn step(
        &mut self, component: &Component, gate: &mut Gate
    ) -> Result<(), Terminated> {
        match gate.process_until(self.fetch_json(component)).await? {
            Ok(Some(res)) => {
                if gate.update(UnitUpdate::Payload(res)).await {
                    debug!(
                        "Unit {}: successfully updated.",
                        component.name()
                    );
                }
                else {
                    debug!(
                        "Unit {}: update without changes.",
                        component.name()
                    );
                }
            }
            Ok(None) => {
                // Fetching succeeded but there isn’t an update. Nothing
                // to do, really.
            }
            Err(Failed) => {
                if gate.update(UnitUpdate::Stalled).await {
                    debug!(
                        "Unit {}: marked as stalled.",
                        component.name()
                    );
                }
            }
        };
        Ok(())
    }

    async fn fetch_json(
        &mut self, component: &Component
    ) -> Result<Option<payload::Update>, Failed> {
        let reader = match HttpReader::open(
            &mut self.uri,
            component,
        ).await? {
            Some(reader) => reader,
            None => {
                debug!("Unit {}: Source not modified.", component.name());
                return Ok(None)
            }
        };
        match spawn_blocking(move || {
            serde_json::from_reader::<_, JsonSet>(reader)
        }).await {
            Ok(Ok(res)) => {
                Ok(Some(payload::Update::new(res.into_payload())))
            }
            Ok(Err(err)) => {
                // Joining succeded but JSON parsing didn’t.
                warn!(
                    "Unit {}: Failed parsing source: {}",
                    component.name(),
                    err
                );
                Err(Failed)
            }
            Err(err) => {
                // Joining failed. This may either be because the JSON
                // parser panicked or because the future was dropped. The
                // former probably means the JSON was kaputt in a very
                // creative way and the latter can’t really happen. So
                // it is probably safe to ignore the JSON as if it were
                // broken.
                if err.is_panic() {
                    warn!(
                        "Unit {}: Failed parsing source: JSON parser panicked.",
                        component.name(),
                    );
                }
                else {
                    warn!(
                        "Unit {}: Failed parsing source: parser was dropped \
                         (This can't happen.)",
                        component.name(),
                    );
                }
                Err(Failed)
            }
        }
    }

    async fn wait(&mut self, gate: &mut Gate) -> Result<(), Terminated> {
        let end = Instant::now() + Duration::from_secs(self.refresh);
        while end > Instant::now() {
            match timeout_at(end, gate.process()).await {
                Ok(Ok(_status)) => { }
                Ok(Err(_)) => return Err(Terminated),
                Err(_) => return Ok(()),
            }
        }

        Ok(())
    }
}


//------------ SourceUri ----------------------------------------------------

/// The URI of the unit’s source.
///
/// This also contains the runtime status for the source which is perhaps a
/// bit cheeky.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "String")]
enum SourceUri {
    Http {
        url: Url,
        last_modified: Option<DateTime<Utc>>,
        etag: Option<Bytes>,
    },
    File {
        path: ConfigPath,
        last_modified: Option<SystemTime>,
    }
}

impl TryFrom<String> for SourceUri {
    type Error = <Url as FromStr>::Err;

    fn try_from(mut src: String) -> Result<Self, Self::Error> {
        if src.starts_with("file:") {
            let src = src.split_off(5);
            Ok(SourceUri::File {
                path: src.into(),
                last_modified: None,
            })
        }
        else {
            let url = Url::from_str(&src)?;
            Ok(SourceUri::Http {
                url,
                last_modified: None,
                etag: None
            })
        }
    }
}


//------------ HttpReader ----------------------------------------------------

struct HttpReader {
    source: ReaderSource,
    chunk: Bytes,
    rt: tokio::runtime::Handle,
}

enum ReaderSource {
    File(File),
    Http(reqwest::Response),
}

impl HttpReader {
    async fn open(
        uri: &mut SourceUri, 
        component: &Component,
    ) -> Result<Option<Self>, Failed> {
        match uri {
            SourceUri::Http {
                ref url, ref mut etag, ref mut last_modified
            } => {
                Self::open_http(url, last_modified, etag, component).await
            }
            SourceUri::File { ref path, ref mut last_modified } => {
                Self::open_file(path, last_modified, component).await
            }
        }
    }

    async fn open_http(
        uri: &Url, 
        last_modified: &mut Option<DateTime<Utc>>,
        etag: &mut Option<Bytes>,
        component: &Component,
    ) -> Result<Option<Self>, Failed> {
        // Create and send the request.
        let mut request = component.http_client().get(uri.clone());
        if let Some(etag) = etag.as_ref() {
            request = request.header(
                header::IF_NONE_MATCH, etag.as_ref()
            );
        }
        if let Some(ts) = last_modified {
            request = request.header(
                header::IF_MODIFIED_SINCE, format_http_date(*ts)
            );
        }
        let response = request.send().await.map_err(|err| {
            warn!(
                "Unit {}: HTTP request failed: {}",
                component.name(), err
            );
            Failed
        })?;

        // Return early if we receive anything other than a 200 OK
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(None)
        }
        else if response.status() != StatusCode::OK {
            warn!(
                "Unit {}: HTTP request return status {}",
                component.name(), response.status()
            );
            return Err(Failed)
        }

        // Update Etag and Last-Modified.
        *etag = Self::parse_etag(&response);
        *last_modified = Self::parse_last_modified(&response);

        // And we are good to go!
        Ok(Some(Self::new(ReaderSource::Http(response))))
    }

    async fn open_file(
        path: &ConfigPath,
        last_modified: &mut Option<SystemTime>,
        component: &Component,
    ) -> Result<Option<Self>, Failed> {
        let modified = metadata(path).and_then(|meta| meta.modified()).ok();
        if let (Some(modified), Some(last_modified)) =
            (modified, last_modified.as_ref())
        {
            if *last_modified >= modified {
                return Ok(None)
            }
        }

        let res = Self::new(
            ReaderSource::File(
                File::open(path).await.map_err(|err| {
                    warn!(
                        "Unit {}: Failed to open file {}: {}.",
                        component.name(), path.display(), err
                    );
                    Failed
                })?
            )
        );

        // Just assigning here should be fine -- if we failed to get the
        // modification time then clearing the stored value is probably a
        // good idea, anyway.
        *last_modified = modified;

        Ok(Some(res))
    }

    fn new(source: ReaderSource) -> Self {
        HttpReader {
            source,
            chunk: Bytes::new(),
            rt: tokio::runtime::Handle::current()
        }
    }

    fn prepare_chunk(&mut self) -> Result<bool, io::Error> {
        if !self.chunk.is_empty() {
            return Ok(true)
        }
        match self.source {
            ReaderSource::File(ref mut file) => {
                let mut buf = BytesMut::with_capacity(16384);
                let read = self.rt.block_on(file.read_buf(&mut buf))?;
                if read == 0 {
                    return Ok(false)
                }
                self.chunk = buf.freeze();
            }
            ReaderSource::Http(ref mut response) => {
                let chunk = self.rt.block_on(response.chunk()).map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("failed to read HTTP response: {}", err)
                    )
                })?;
                self.chunk = match chunk {
                    Some(chunk) => chunk,
                    None => return Ok(false)
                };
            }
        }
        Ok(true)
    }

    fn parse_etag(response: &reqwest::Response) -> Option<Bytes> {
        // Take the value of the first Etag header. Return None if there’s
        // more than one, just to be safe.
        let mut etags = response.headers()
            .get_all(header::ETAG)
            .into_iter();
        let etag = etags.next()?;
        if etags.next().is_some() {
            return None
        }
        let etag = etag.as_bytes();

        // The tag starts with an optional case-sensitive `W/` followed by
        // `"`. Let’s remember where the actual tag starts.
        let start = if etag.starts_with(b"W/\"") {
            3
        }
        else if etag.first() == Some(&b'"') {
            1
        }
        else {
            return None
        };

        // We need at least one more character. Empty tags are allowed.
        if etag.len() <= start {
            return None
        }

        // The tag ends with a `"`.
        if etag.last() != Some(&b'"') {
            return None
        }

        Some(Bytes::copy_from_slice(etag))
    }

    fn parse_last_modified(
        response: &reqwest::Response
    ) -> Option<DateTime<Utc>> {
        let mut iter = response.headers()
            .get_all(header::LAST_MODIFIED)
            .into_iter();
        let value = iter.next()?;
        if iter.next().is_some() {
            return None
        }
        parse_http_date(value.to_str().ok()?)
    }
}

impl io::Read for HttpReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        if !self.prepare_chunk()? {
            return Ok(0)
        }

        let len = cmp::min(self.chunk.len(), buf.len());
        buf[..len].copy_from_slice(&self.chunk[..len]);
        self.chunk.advance(len);
        Ok(len)
    }
}

