//! RTR Clients.

use std::io;
use std::sync::Arc;
use std::time::Duration;
use futures::pin_mut;
use futures::future::{select, Either};
use log::{debug, warn};
use rpki::rtr::client::{Client, PayloadError, PayloadTarget, PayloadUpdate};
use rpki::rtr::payload::{Action, Payload, Timing};
use rpki::rtr::state::{Serial, State};
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio::time::{timeout_at, Instant};
use crate::metrics;
use crate::comms::{Gate, GateMetrics, GateStatus, Terminated, UnitStatus};
use crate::manager::Component;
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
        mut self, mut component: Component, mut gate: Gate
    ) -> Result<(), Terminated> {
        let mut target = Target::new(component.name().clone());
        let metrics = Arc::new(RtrMetrics::new(&gate));
        component.register_metrics(metrics.clone());
        gate.update_status(UnitStatus::Stalled).await;
        loop {
            debug!("Unit {}: Connecting ...", target.name);
            let mut client = match self.connect(target, &mut gate).await {
                Ok(client) => {
                    gate.update_status(UnitStatus::Healthy).await;
                    client
                }
                Err(res) => {
                    debug!(
                        "Unit {}: Connection failed. Awaiting reconnect.",
                        res.name
                    );
                    gate.update_status(UnitStatus::Stalled).await;
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
                        debug!(
                            "Unit {}: RTR client disconnected.",
                            client.target().name
                        );
                        break;
                    }
                    Err(_) => {
                        debug!(
                            "Unit {}: RTR client terminated.",
                            client.target().name
                        );
                        return Err(Terminated)
                    }
                };
                if let Some(update) = update {
                    self.serial = update.serial();
                    client.target_mut().current = update.set().clone();
                    gate.update_data(update).await;
                }
            }

            target = client.into_target();
            gate.update_status(UnitStatus::Stalled).await;
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
    ) -> Result<Result<Option<payload::Update>, io::Error>, Terminated> {
        let next_serial = self.serial.add(1);
        let update_fut = async {
            let update = client.update().await?;
            if update.is_definitely_empty() {
                return Ok(None)
            }
            match update.into_update(next_serial) {
                Ok(res) => Ok(Some(res)),
                Err(err) => {
                    client.send_error(err).await?;
                    Err(io::Error::new(io::ErrorKind::Other, err))
                }
            }
        };
        pin_mut!(update_fut);

        loop {
            let process = gate.process();
            pin_mut!(process);
            match select(process, update_fut).await {
                Either::Left((Err(_), _)) => {
                    return Err(Terminated)
                }
                Either::Left((Ok(status), next_fut)) => {
                    self.status = status;
                    update_fut = next_fut;
                }
                Either::Right((res, _)) => {
                    return Ok(res)
                }
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
    current: payload::Set,

    state: Option<State>,

    name: Arc<str>,
}

impl Target {
    pub fn new(name: Arc<str>) -> Self {
        Target {
            current: Default::default(),
            state: None,
            name
        }
    }
}

impl PayloadTarget for Target {
    type Update = TargetUpdate;

    fn start(&mut self, reset: bool) -> Self::Update {
        debug!("Unit {}: starting update (reset={})", self.name, reset);
        if reset {
            TargetUpdate::Reset(payload::PackBuilder::empty())
        }
        else {
            TargetUpdate::Serial {
                set: self.current.clone(),
                diff: payload::DiffBuilder::empty(),
            }
        }
    }

    fn apply(
        &mut self, 
        _update: Self::Update, 
        _timing: Timing
    ) -> Result<(), PayloadError> {
        unreachable!()
    }
}


//------------ TargetUpdate --------------------------------------------------

enum TargetUpdate {
    Reset(payload::PackBuilder),
    Serial {
        set: payload::Set,
        diff: payload::DiffBuilder,
    }
}

impl TargetUpdate {
    fn is_definitely_empty(&self) -> bool {
        match *self {
            TargetUpdate::Reset(_) => false,
            TargetUpdate::Serial { ref diff, .. } => diff.is_empty()
        }
    }

    fn into_update(
        self, serial: Serial
    ) -> Result<payload::Update, PayloadError> {
        match self {
            TargetUpdate::Reset(pack) => {
                Ok(payload::Update::new(serial, pack.finalize().into(), None))
            }
            TargetUpdate::Serial { set, diff } => {
                let diff = diff.finalize();
                let set = diff.apply(&set)?;
                Ok(payload::Update::new(serial, set, Some(diff)))
            }
        }
    }
}

impl PayloadUpdate for TargetUpdate {
    fn push_update(
        &mut self, 
        action: Action, 
        payload: Payload
    ) -> Result<(), PayloadError> {
        match *self {
            TargetUpdate::Reset(ref mut pack) => {
                if action == Action::Withdraw {
                    Err(PayloadError::Corrupt)
                }
                else {
                    pack.insert(payload)
                }
            }
            TargetUpdate::Serial { ref mut diff, .. } => {
                diff.push(payload, action)
            }
        }
    }
}


//------------ RtrMetrics ----------------------------------------------------

#[derive(Debug, Default)]
struct RtrMetrics {
    gate: Arc<GateMetrics>,
}

impl RtrMetrics {
    fn new(gate: &Gate) -> Self {
        RtrMetrics {
            gate: gate.metrics(),
        }
    }
}

impl metrics::Source for RtrMetrics {
    fn append(&self, unit_name: &str, target: &mut metrics::Target)  {
        self.gate.append(unit_name, target);
    }
}

