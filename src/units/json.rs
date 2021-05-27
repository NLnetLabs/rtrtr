//! JSON clients.

use std::{io, thread};
use std::fs::File;
use std::time::Duration;
use log::{debug, error, warn};
use reqwest::Url;
use rpki::rtr::Serial;
use serde::Deserialize;
use tokio::sync::oneshot;
use tokio::time::{Instant, timeout_at};
use crate::payload;
use crate::comms::{Gate, Terminated, UnitStatus};
use crate::formats::json::Set as JsonSet;
use crate::manager::Component;

//------------ Json ----------------------------------------------------------

/// An unit that regularly fetches a JSON-encoded VRP set.
#[derive(Debug, Deserialize)]
pub struct Json {
    /// The URI of the JSON source.
    uri: Url,

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
}

impl JsonRunner {
    fn new(
        json: Json, component: Component, gate: Gate
    ) -> Self {
        JsonRunner {
            json, component, gate,
            serial: Serial::default(),
            status: UnitStatus::Stalled,
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
        if self.json.uri.scheme() == "file" {
            self.step_file().await
        }
        else if
            self.json.uri.scheme() == "http"
            || self.json.uri.scheme() == "https"
        {
            self.step_http().await
        }
        else {
            error!(
                "{}: Cannot resolve URI '{}'",
                self.component.name(), self.json.uri
            );
            Err(Terminated)
        }
    }

    async fn step_file(&mut self) -> Result<(), Terminated> {
        debug!("Unit {}: Updating from {}",
            self.component.name(), self.json.uri
        );
        let uri = self.json.uri.clone();
        if let Err(err) = self.step_generic(
            move || File::open(uri.path())
        ).await? {
            warn!("{}: cannot open file '{}': {}",
                self.component.name(),
                self.json.uri.path(),
                err
            );
        }
        Ok(())
    }

    async fn step_http(&mut self) -> Result<(), Terminated> {
        debug!("Unit {}: Updating from {}",
            self.component.name(), self.json.uri
        );
        let request = self.component.http_client().get(self.json.uri.clone());
        if let Err(err) = self.step_generic(move || request.send()).await? {
            warn!("{}: failed to fetch from '{}': {}",
                self.component.name(),
                self.json.uri,
                err
            );
        }
        Ok(())
    }

    async fn step_generic<F, R, E>(
        &mut self, op: F
    ) -> Result<Result<(), E>, Terminated>
    where
        F: FnOnce() -> Result<R, E> + Send + 'static,
        R: io::Read,
        E: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let _ = thread::spawn(move || {
            let reader = match op() {
                Ok(reader) => reader,
                Err(err)=> {
                    let _ = tx.send(Err(err));
                    return;
                }
            };
            let res = serde_json::from_reader::<_, JsonSet>(reader);
            let _ = tx.send(Ok(res));
        });

        // XXX I think awaiting rx should never produce an error, so
        //     unwrapping is the right thing to do. But is it really?
        let res = match self.gate.process_until(rx).await?.unwrap() {
            Ok(Ok(res)) => res,
            Ok(Err(err)) => {
                if self.status != UnitStatus::Stalled {
                    self.status = UnitStatus::Stalled;
                    self.gate.update_status(self.status).await
                }
                warn!(
                    "{}: Failed reading source: {}",
                    self.component.name(),
                    err
                );
                return Ok(Ok(()))
            }
            Err(err) => return Ok(Err(err))
        };
        
        self.serial = self.serial.add(1);
        if self.status != UnitStatus::Healthy {
            self.status = UnitStatus::Healthy;
            self.gate.update_status(self.status).await
        }
        self.gate.update_data(
            payload::Update::new(self.serial, res.into_payload().into(), None)
        ).await;
        debug!("Unit {}: successfully updated.", self.component.name());
        Ok(Ok(()))
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

