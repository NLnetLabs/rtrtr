//! RTR Clients.

use std::io;
use std::sync::Arc;
use std::time::Duration;
use futures::pin_mut;
use futures::future::{select, Either};
use log::{debug, warn};
use rpki_rtr::client::{Client, VrpError, VrpTarget, VrpUpdate};
use rpki_rtr::payload::{Action, Payload, Timing};
use rpki_rtr::state::{Serial, State};
use serde_derive::Deserialize;
use tokio::net::TcpStream;
use tokio::time::{timeout_at, Instant};
use crate::comms::{Gate, GateStatus, Terminated};
use crate::payload;


//------------ Tcp -----------------------------------------------------------

/// An RTR client using an unencrypted plain TCP socket.
#[derive(Debug, Deserialize)]
pub struct Tcp {
    /// The remote address to connect to.
    remote: String,

    /// How long to wait before connecting again if the connection is closed.
    #[serde(default = "Tcp::default_retry")]
    retry: u64,

    /// Our gate status.
    #[serde(skip)]
    status: GateStatus,

    /// Our current serial.
    #[serde(skip)]
    serial: Serial,
}

impl Tcp {
    pub fn default_retry() -> u64 {
        60
    }

    pub async fn run(
        mut self, name: String, mut gate: Gate
    ) -> Result<(), Terminated> {
        let mut target = Target::new(name);
        loop {
            let mut client = match self.connect(target, &mut gate).await {
                Ok(client) => client,
                Err(res) => {
                    self.retry_wait(&mut gate).await?;
                    target = res;
                    continue;
                }
            };

            loop {
                let update = match self.update(&mut client, &mut gate).await {
                    Ok(Ok(update)) => {
                        debug!(
                            "Unit {}: received update.", client.target().name
                        );
                        update
                    }
                    Ok(Err(_)) => {
                        // XXX Log?
                        break;
                    }
                    Err(_) => return Err(Terminated)
                };
                self.serial = self.serial.add(1);
                let update = update.into_update(self.serial);
                gate.update_data(update).await;
            }

            target = client.into_target();
            self.retry_wait(&mut gate).await?;
        }
    }

    async fn connect(
        &mut self, target: Target, gate: &mut Gate,
    ) -> Result<Client<TcpStream, Target>, Target> {
        let sock = {
            let connect = TcpStream::connect(&self.remote);
            pin_mut!(connect);
            
            loop {
                let process = gate.process();
                pin_mut!(process);
                match select(process, connect).await {
                    Either::Left((Err(_), _)) => {
                        return Err(target)
                    }
                    Either::Left((Ok(status), next_fut)) => {
                        self.status = status;
                        connect = next_fut;
                    }
                    Either::Right((res, _)) => break res
                }
            }
        };

        let sock = match sock {
            Ok(sock) => sock,
            Err(err) => {
                warn!(
                    "Unit {}: Failed to connect to RTR server {}: {}",
                    target.name, &self.remote, err
                );
                return Err(target)
            }
        };

        let state = target.state;
        Ok(Client::new(sock, target, state))
    }

    async fn update(
        &mut self, client: &mut Client<TcpStream, Target>, gate: &mut Gate
    ) -> Result<Result<TargetUpdate, io::Error>, Terminated> {
        let update = client.update();
        pin_mut!(update);

        loop {
            let process = gate.process();
            pin_mut!(process);
            match select(process, update).await {
                Either::Left((Err(_), _)) => {
                    return Err(Terminated)
                }
                Either::Left((Ok(status), next_fut)) => {
                    self.status = status;
                    update = next_fut;
                }
                Either::Right((res, _)) => return Ok(res)
            }
        }
    }

    async fn retry_wait(
        &mut self, gate: &mut Gate
    ) -> Result<(), Terminated> {
        let end = Instant::now() + Duration::from_secs(self.retry);

        while end > Instant::now() {
            match timeout_at(end, gate.process()).await {
                Ok(Ok(status)) => {
                    self.status = status
                }
                Ok(Err(_)) => return Err(Terminated),
                Err(_) => return Ok(()),
            }
        }

        Ok(())
    }
}


//------------ Target --------------------------------------------------------

struct Target {
    current: Arc<payload::Set>,

    state: Option<State>,

    name: String,
}

impl Target {
    pub fn new(name: String) -> Self {
        Target {
            current: Default::default(),
            state: None,
            name
        }
    }
}

impl VrpTarget for Target {
    type Update = TargetUpdate;

    fn start(&mut self, reset: bool) -> Self::Update {
        debug!("Unit {}: starting update (reset={})", self.name, reset);
        TargetUpdate {
            set: self.current.as_ref().into(),
            diff: if reset {
                None
            }
            else {
                Some(Default::default())
            }
        }
    }

    fn apply(
        &mut self, 
        _update: Self::Update, 
        _reset: bool, 
        _timing: Timing
    ) -> Result<(), VrpError> {
        unreachable!()
    }
}


//------------ TargetUpdate --------------------------------------------------

struct TargetUpdate {
    /// The new data set.
    set: payload::SetBuilder,

    /// The diff.
    ///
    /// If this is `None` we are processing a reset query.
    diff: Option<payload::DiffBuilder>,
}

impl TargetUpdate {
    fn into_update(self, serial: Serial) -> payload::Update {
        payload::Update::new(
            serial,
            Arc::new(self.set.finalize()),
            self.diff.map(|diff| Arc::new(diff.finalize()))
        )
    }
}

impl VrpUpdate for TargetUpdate {
    fn push_vrp(
        &mut self, 
        action: Action, 
        payload: Payload
    ) -> Result<(), VrpError> {
        match self.diff {
            Some(ref mut diff) => {
                match action {
                    Action::Announce => {
                        self.set.insert(payload.clone())?;
                    }
                    Action::Withdraw => {
                        self.set.remove(&payload)?;
                    }
                }
                diff.push(payload, action)
            }
            None => {
                if action == Action::Withdraw {
                    Err(VrpError::Corrupt)
                }
                else {
                    self.set.insert(payload)
                }
            }
        }
    }
}

