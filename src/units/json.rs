//! JSON clients.

use std::{cmp, io};
use std::str::FromStr;
use std::time::Duration;
//use chrono::{DateTime, Utc};
use bytes::{Buf, Bytes, BytesMut};
use log::{debug, warn};
use reqwest::Url;
use rpki::rtr::Serial;
use serde::Deserialize;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::task::spawn_blocking;
use tokio::time::{Instant, timeout_at};
use crate::payload;
use crate::comms::{Gate, Terminated, UnitStatus};
use crate::config::ConfigPath;
use crate::formats::json::Set as JsonSet;
use crate::manager::Component;
use crate::log::Failed;


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
        self, component: Component, gate: Gate
    ) -> Result<(), Terminated> {
        JsonRunner::new(self, component).run(gate).await
    }
}


//----------- JsonRunner -----------------------------------------------------

struct JsonRunner {
    json: Json,
    component: Component,
    serial: Serial,
    status: UnitStatus,
    current: Option<payload::Set>,
            /*
    last_modified: Option<DateTime<Utc>>,
    etag: Option<String>,
            */
}

impl JsonRunner {
    fn new(
        json: Json, component: Component
    ) -> Self {
        JsonRunner {
            json, component,
            serial: Serial::default(),
            status: UnitStatus::Stalled,
            current: Default::default(),
            /*
            last_modified: None,
            etag: None,
            */
        }
    }

    async fn run(mut self, mut gate: Gate) -> Result<(), Terminated> {
        self.component.register_metrics(gate.metrics());
        gate.update_status(self.status).await;
        loop {
            self.step(&mut gate).await?;
            self.wait(&mut gate).await?;
        }
    }

    async fn step(&mut self, gate: &mut Gate) -> Result<(), Terminated> {
        match gate.process_until(self.fetch_json()).await? {
            Ok(res) => {
                let res = res.into_payload();
                if self.current.as_ref() != Some(&res) {
                    self.serial = self.serial.add(1);
                    self.current = Some(res.clone());
                    if self.status != UnitStatus::Healthy {
                        self.status = UnitStatus::Healthy;
                        gate.update_status(self.status).await
                    }
                    gate.update_data(
                        payload::Update::new(self.serial, res, None)
                    ).await;
                    debug!(
                        "Unit {}: successfully updated.",
                        self.component.name()
                    );
                }
                else {
                    debug!(
                        "Unit {}: update without changes.",
                        self.component.name()
                    );
                }
            }
            Err(Failed) => {
                if self.status != UnitStatus::Stalled {
                    self.status = UnitStatus::Stalled;
                    gate.update_status(self.status).await
                }
                debug!("Unit {}: marked as stalled.", self.component.name());
            }
        };
        Ok(())
    }

    async fn fetch_json(&mut self) -> Result<JsonSet, Failed> {
        let reader = HttpReader::new(match self.json.uri {
            SourceUri::Http(ref url) => {
                ReaderSource::Http(
                    self.component.http_client().get(
                        url.clone()
                    ).send().await.map_err(|err| {
                        warn!(
                            "Unit {}: HTTP request failed: {}",
                            self.component.name(), err
                        );
                        Failed
                    })?
                )
            }
            SourceUri::File(ref path) => {
                match File::open(path).await {
                    Ok(file) => ReaderSource::File(file),
                    Err(err) => {
                        warn!(
                            "Unit {}: Failed to open file {}: {}.",
                            self.component.name(), path.display(), err
                        );
                        return Err(Failed)
                    }
                }
            }
        });
        match spawn_blocking(move || {
            serde_json::from_reader::<_, JsonSet>(reader)
        }).await {
            Ok(Ok(res)) => Ok(res),
            Ok(Err(err)) => {
                // Joining succeded but JSON parsing didn’t.
                warn!(
                    "{}: Failed parsing source: {}",
                    self.component.name(),
                    err
                );
                Err(Failed)
            }
            Err(err) => {
                // Joining failed. This may either be because the JSON
                // parser panicked or because the future was dropped. The
                // former probably means the JSON was kaputt in a very
                // creative way and the latter can’t really happening. So
                // it is probably safe to ignore the JSON as if it were
                // broken.
                if err.is_panic() {
                    warn!(
                        "Unit {}: Failed parsing source: JSON parser panicked.",
                        self.component.name(),
                    );
                }
                else {
                    warn!(
                        "Unit {}: Failed parsing source: parser was dropped \
                         (This can't happen.)",
                        self.component.name(),
                    );
                }
                Err(Failed)
            }
        }
    }

    async fn wait(&mut self, gate: &mut Gate) -> Result<(), Terminated> {
        let end = Instant::now() + Duration::from_secs(self.json.refresh);
        while end > Instant::now() {
            match timeout_at(end, gate.process()).await {
                Ok(Ok(_status)) => {
                    //self.status = status
                }
                Ok(Err(_)) => return Err(Terminated),
                Err(_) => return Ok(()),
            }
        }

        Ok(())
    }
}


//------------ SourceUri ----------------------------------------------------

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
            let src = src.split_off(5);
            Ok(SourceUri::File(src.into()))
        }
        else {
            Url::from_str(&src).map(SourceUri::Http)
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

