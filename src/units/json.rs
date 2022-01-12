//! JSON clients.

use std::{io, thread};
use std::convert::TryFrom;
use std::fs::File;
use std::str::FromStr;
use std::time::Duration;
use log::{debug, warn};
use reqwest::Url;
use rpki::rtr::Serial;
use serde::Deserialize;
use tokio::sync::oneshot;
use tokio::time::{Instant, timeout_at};
use crate::payload;
use crate::comms::{Gate, Terminated, UnitStatus};
use crate::config::ConfigPath;
use crate::formats::json::Set as JsonSet;
use crate::manager::Component;

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
        JsonRunner::new(self, component, gate).run().await
    }
}


//----------- JsonRunner -----------------------------------------------------

struct JsonRunner {
    json: Json,
    component: Component,
    gate: Gate,
    serial: Serial,
    status: UnitStatus,
    current: Option<payload::Set>,
}

impl JsonRunner {
    fn new(
        json: Json, component: Component, gate: Gate
    ) -> Self {
        JsonRunner {
            json, component, gate,
            serial: Serial::default(),
            status: UnitStatus::Stalled,
            current: Default::default(),
        }
    }

    async fn run(mut self) -> Result<(), Terminated> {
        self.component.register_metrics(self.gate.metrics());
        self.gate.update_status(self.status).await;
        loop {
            self.step().await?;
            self.wait().await?;
        }
    }

    async fn step(&mut self) -> Result<(), Terminated> {
        match self.load_json().await? {
            Some(res) => {
                let res = res.into_payload();
                if self.current.as_ref() != Some(&res) {
                    self.serial = self.serial.add(1);
                    self.current = Some(res.clone());
                    if self.status != UnitStatus::Healthy {
                        self.status = UnitStatus::Healthy;
                        self.gate.update_status(self.status).await
                    }
                    self.gate.update_data(
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
            None => {
                if self.status != UnitStatus::Stalled {
                    self.status = UnitStatus::Stalled;
                    self.gate.update_status(self.status).await
                }
                debug!("Unit {}: marked as stalled.", self.component.name());
            }
        };
        Ok(())
    }

    async fn load_json(&mut self) -> Result<Option<JsonSet>, Terminated> {
        let (tx, rx) = oneshot::channel();
        let reader = match self.json.uri.reader(&self.component) {
            Some(reader) => reader,
            None => return Ok(None)
        };
        let _ = thread::spawn(move || {
            let _ = tx.send(serde_json::from_reader::<_, JsonSet>(reader));
        });

        // XXX I think awaiting rx should never produce an error, so
        //     unwrapping is the right thing to do. But is it really?
        match self.gate.process_until(rx).await?.unwrap() {
            Ok(res) => Ok(Some(res)),
            Err(err) => {
                warn!(
                    "{}: Failed parsing source: {}",
                    self.component.name(),
                    err
                );
                Ok(None)
            }
        }
    }

    async fn wait(&mut self) -> Result<(), Terminated> {
        let end = Instant::now() + Duration::from_secs(self.json.refresh);
        while end > Instant::now() {
            match timeout_at(end, self.gate.process()).await {
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

impl SourceUri {
    fn reader(&self, component: &Component) -> Option<JsonReader> {
        match *self {
            SourceUri::Http(ref uri) => {
                Some(JsonReader::HttpRequest(
                    Some(component.http_client().get(uri.clone()))
                ))
            }
            SourceUri::File(ref path) => {
                match File::open(path).map(JsonReader::File) {
                    Ok(some) => Some(some),
                    Err(err) => {
                        warn!(
                            "{}: Failed reading open {}: {}",
                            component.name(),
                            path.display(),
                            err
                        );
                        None
                    }
                }
            }
        }
    }
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


//------------ JsonReader ----------------------------------------------------

/// A reader producing the JSON source.
enum JsonReader {
    File(File),
    HttpRequest(Option<reqwest::blocking::RequestBuilder>),
    Http(reqwest::blocking::Response),
}

impl io::Read for JsonReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        let http = match *self {
            JsonReader::File(ref mut inner) => {
                return inner.read(buf)
            }
            JsonReader::Http(ref mut inner) => {
                return inner.read(buf)
            }
            JsonReader::HttpRequest(ref mut inner) => {
                match inner.take() {
                    Some(inner) => {
                        inner.send().map_err(|err| {
                            io::Error::new(
                                io::ErrorKind::Other,
                                err
                            )
                        })?
                    }
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            "already failed to send request"
                        ))
                    }
                }
            }
        };
        *self = JsonReader::Http(http);
        self.read(buf)
    }
}

