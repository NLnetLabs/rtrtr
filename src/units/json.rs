//! JSON clients.

use std::{cmp, fs, io};
use std::fs::metadata;
use std::str::FromStr;
use std::time::{Duration, SystemTime};
use chrono::{DateTime, Utc};
use bytes::{Buf, Bytes, BytesMut};
use daemonbase::config::ConfigPath;
use daemonbase::error::Failed;
use log::{debug, error, warn};
use reqwest::{header, tls};
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

/// A unit that regularly fetches a JSON-encoded VRP set.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Json {
    /// The URI of the JSON source.
    uri: SourceUri,

    /// How many seconds to wait before refreshing the data.
    refresh: u64,

    /// Path to a file with a client certificate and private key.
    #[serde(default, deserialize_with = "deserialize_identity")]
    identity: Option<ConfigPath>,

    /// Use the native-tls backend.
    #[cfg(feature = "native-tls")]
    #[serde(default)]
    native_tls: bool,

    /// Only use TLS up to version 1.2.
    #[serde(default)]
    tls_12: bool,
}

impl Json {
    pub async fn run(
        self, mut component: Component, mut gate: Gate
    ) -> Result<(), Terminated> {
        component.register_metrics(gate.metrics());
        let mut source = self.create_source(&component)?;
        loop {
            self.step(&mut source, &component, &mut gate).await?;
            self.wait(&mut gate).await?;
        }
    }

    fn create_source(
        &self, component: &Component
    ) -> Result<Source, Terminated> {
        match self.uri {
            SourceUri::Http(ref url) => {
                Ok(Source::Http {
                    url,
                    client: self.http_client(component)?,
                    last_modified: None,
                    etag: None,
                })
            }
            SourceUri::File(ref path) => {
                Ok(Source::File { path, last_modified: None })
            }
        }
    }

    fn http_client(
        &self, component: &Component
    ) -> Result<reqwest::Client, Terminated> {
        let mut builder = component.http_client().map_err(|err| {
            error!("Unit {}: {}", component.name(), err);
            Terminated
        })?;

        #[cfg(feature = "native-tls")]
        if self.native_tls {
            builder = builder.use_native_tls();
        }

        if self.tls_12 {
            builder = builder.max_tls_version(
                tls::Version::TLS_1_2
            );
        }

        if let Some(identity) = self.identity.as_ref() {
            let data = fs::read(identity).map_err(|err| {
                error!("Unit {}: cannot read identity file {}: {}",
                    component.name(), identity.display(), err
                );
                Terminated
            })?;
            let identity = self.load_identity(&data, component)?;
            builder = builder.identity(identity);
            debug!("Unit {}: successfully loaded client certificate.",
                component.name()
            );
        }
        builder.build().map_err(|err| {
            error!("Unit {}: Failed to initialize HTTP client: {}.",
                component.name(), err
            );
            Terminated
        })
    }

    #[cfg(not(feature = "native-tls"))]
    fn load_identity(
        &self, data: &[u8], component: &Component
    ) -> Result<tls::Identity, Terminated> {
        tls::Identity::from_pem(data).map_err(|err| {
            error!("Unit {}: cannot parse rustls TLS identity file: {:?}",
                component.name(), err
            );
            Terminated
        })
    }

    #[cfg(feature = "native-tls")]
    fn load_identity(
        &self, data: &[u8], component: &Component
    ) -> Result<tls::Identity, Terminated> {
        tls::Identity::from_pkcs8_pem(data, data).map_err(|err| {
            error!("Unit {}: cannot parse native identity file: {:?}",
                component.name(), err
            );
            Terminated
        })
    }

    async fn step(
        &self,
        source: &mut Source<'_>,
        component: &Component,
        gate: &mut Gate
    ) -> Result<(), Terminated> {
        match gate.process_until(self.fetch_json(source, component)).await? {
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
        &self, source: &mut Source<'_>, component: &Component
    ) -> Result<Option<payload::Update>, Failed> {
        let reader = match SourceReader::open(source, component).await? {
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

    async fn wait(&self, gate: &mut Gate) -> Result<(), Terminated> {
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


//------------ SourceUri -----------------------------------------------------

/// The URI of the unit’s source.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "String")]
enum SourceUri {
    Http(Url),
    File(ConfigPath),
}

impl TryFrom<String> for SourceUri {
    type Error = <Url as FromStr>::Err;

    fn try_from(mut src: String) -> Result<Self, Self::Error> {
        if src.starts_with("file:") {
            Ok(SourceUri::File(src.split_off(5).into()))
        }
        else {
            Ok(SourceUri::Http(Url::from_str(&src)?))
        }
    }
}


//------------ Source --------------------------------------------------------

/// Information about the data source of the unit.
#[derive(Clone, Debug)]
enum Source<'a> {
    Http {
        url: &'a Url,
        client: reqwest::Client,
        last_modified: Option<DateTime<Utc>>,
        etag: Option<Bytes>,
    },
    File {
        path: &'a ConfigPath,
        last_modified: Option<SystemTime>,
    }
}


//------------ SourceReader ----------------------------------------------------

struct SourceReader {
    reader: Reader,
    chunk: Bytes,
    rt: tokio::runtime::Handle,
}

enum Reader {
    File(File),
    Http(reqwest::Response),
}

impl SourceReader {
    async fn open(
        source: &mut Source<'_>, 
        component: &Component,
    ) -> Result<Option<Self>, Failed> {
        match source {
            Source::Http {
                url, ref client, ref mut etag, ref mut last_modified
            } => {
                Self::open_http(
                    url, client, last_modified, etag, component
                ).await
            }
            Source::File { path, ref mut last_modified } => {
                Self::open_file(path, last_modified, component).await
            }
        }
    }

    async fn open_http(
        uri: &Url, 
        client: &reqwest::Client,
        last_modified: &mut Option<DateTime<Utc>>,
        etag: &mut Option<Bytes>,
        component: &Component,
    ) -> Result<Option<Self>, Failed> {
        // Create and send the request.
        let mut request = client.get(uri.clone());
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
        Ok(Some(Self::new(Reader::Http(response))))
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
            Reader::File(
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

    fn new(reader: Reader) -> Self {
        SourceReader {
            reader,
            chunk: Bytes::new(),
            rt: tokio::runtime::Handle::current()
        }
    }

    fn prepare_chunk(&mut self) -> Result<bool, io::Error> {
        if !self.chunk.is_empty() {
            return Ok(true)
        }
        match self.reader{
            Reader::File(ref mut file) => {
                let mut buf = BytesMut::with_capacity(16384);
                let read = self.rt.block_on(file.read_buf(&mut buf))?;
                if read == 0 {
                    return Ok(false)
                }
                self.chunk = buf.freeze();
            }
            Reader::Http(ref mut response) => {
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

impl io::Read for SourceReader {
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


//------------ Helper Functions ------------------------------------------------

fn deserialize_identity<'de, D: serde::Deserializer<'de>>(
    deserializer: D
) -> Result<Option<ConfigPath>, D::Error> {
    ConfigPath::deserialize(deserializer).map(Some)
}

