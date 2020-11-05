/// RTR servers as a target.

use std::cmp;
use std::sync::Arc;
use std::net::SocketAddr;
use std::net::TcpListener as StdTcpListener;
use arc_swap::ArcSwap;
use log::{debug, error};
use serde_derive::Deserialize;
use rpki_rtr::payload::Timing;
use rpki_rtr::server::{NotifySender, Server, VrpSource};
use rpki_rtr::state::{Serial, State};
use tokio::net::TcpListener;
use crate::log::ExitError;
use crate::payload;
use crate::comms::Link;


//------------ Tcp -----------------------------------------------------------

/// An RTR server atop unencrypted, plain TCP.
#[derive(Debug, Deserialize)]
pub struct Tcp {
    listen: Vec<SocketAddr>,
    unit: Link,
}

impl Tcp {
    pub async fn run(mut self, name: String) -> Result<(), ExitError> {
        let mut notify = NotifySender::new();
        let target = Source::default();
        for &addr in &self.listen {
            self.spawn_listener(addr, target.clone(), notify.clone())?;
        }

        loop {
            if let Ok(update) = self.unit.query().await {
                debug!(
                    "Target {}: Got update ({} entries)",
                    name, update.set().len()
                );
                target.update(update);
                notify.notify()
            }
        }
    }

    fn spawn_listener(
        &self, addr: SocketAddr, target: Source, notify: NotifySender,
    ) -> Result<(), ExitError> {
        let listener = match StdTcpListener::bind(addr) {
            Ok(listener) => listener,
            Err(err) => {
                error!("Can’t bind to {}: {}", addr, err);
                return Err(ExitError)
            }
        };
        let mut listener = match TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(err) => {
                error!("Fatal error listening on {}: {}", addr, err);
                return Err(ExitError)
            }
        };
        tokio::spawn(async move {
            let listener = listener.incoming();
            let server = Server::new(listener, notify, target);
            if server.run().await.is_err() {
                error!("Fatal error listening on {}.", addr);
            }
        });   
        Ok(())
    }

}


//------------ Source --------------------------------------------------------

#[derive(Clone, Default)]
struct Source {
    data: ArcSwap<SourceData>,
    diff_num: usize,
}

impl Source {
    fn update(&self, update: payload::Update) {
        let data = self.data.load();

        let new_data = match data.current.as_ref() {
            None => {
                SourceData {
                    state: data.state,
                    unit_serial: update.serial(),
                    current: Some(update.set()),
                    diffs: Vec::new(),
                    timing: Timing::default(),
                }
            }
            Some(current) => {
                let diff = match update.get_usable_diff(data.unit_serial) {
                    Some(diff) => diff,
                    None => Arc::new(update.set().diff_from(current)),
                };
                if diff.is_empty() {
                    // If there is no change in data, don’t update.
                    return
                }
                let mut diffs = Vec::with_capacity(
                    cmp::min(data.diffs.len() + 1, self.diff_num)
                );
                diffs.push((data.state.serial(), diff.clone()));
                for (serial, old_diff) in &data.diffs {
                    if diffs.len() == self.diff_num {
                        break
                    }
                    diffs.push((
                        *serial,
                        Arc::new(old_diff.extend(&diff).unwrap())
                    ))
                }
                let mut state = data.state;
                state.inc();
                SourceData {
                    state,
                    unit_serial: update.serial(),
                    current: Some(update.set()),
                    diffs,
                    timing: Timing::default(),
                }
            }
        };

        self.data.store(new_data.into());
    }
}

impl VrpSource for Source {
    type FullIter = payload::SetIter;
    type DiffIter = payload::DiffIter;

    fn ready(&self) -> bool {
        self.data.load().current.is_some()
    }

    fn notify(&self) -> State {
        self.data.load().state
    }

    fn full(&self) -> (State, Self::FullIter) {
        let this = self.data.load();
        match this.current.as_ref() {
            Some(current) => (this.state, current.clone().into()),
            None => (this.state, Arc::new(payload::Set::default()).into())
        }
    }

    fn diff(&self, state: State) -> Option<(State, Self::DiffIter)> {
        let this = self.data.load();
        if this.current.is_none() || state.session() != this.state.session() {
            return None
        }

        this.get_diff(state.serial()).map(|diff| {
            (this.state, diff.shared_iter())
        })
    }

    fn timing(&self) -> Timing {
        self.data.load().timing
    }
}


//------------ SourceData ----------------------------------------------------

#[derive(Clone, Default)]
struct SourceData {
    /// The current RTR state of the target.
    state: State,

    /// The current serial of the source unit.
    unit_serial: Serial,

    /// The current set of RTR data.
    current: Option<Arc<payload::Set>>,

    /// The diffs we currently keep.
    ///
    /// The diff with the largest serial is first.
    diffs: Vec<(Serial, Arc<payload::Diff>)>,

    /// The timing paramters for this source.
    timing: Timing,
}

impl SourceData {
    fn get_diff(&self, serial: Serial) -> Option<Arc<payload::Diff>> {
        if serial == self.state.serial() {
            Some(Arc::new(payload::Diff::default()))
        }
        else {
            self.diffs.iter().find_map(|item| {
                if item.0 == serial {
                    Some(item.1.clone())
                }
                else {
                    None
                }
            })
        }
    }
}

