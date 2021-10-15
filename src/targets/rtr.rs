/// RTR servers as a target.

use std::cmp;
use std::sync::Arc;
use std::net::SocketAddr;
use std::net::TcpListener as StdTcpListener;
use arc_swap::ArcSwap;
use log::{debug, error};
use serde::Deserialize;
use rpki::rtr::payload::Timing;
use rpki::rtr::server::{NotifySender, Server, PayloadSource};
use rpki::rtr::state::{Serial, State};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use crate::payload;
use crate::comms::Link;
use crate::log::ExitError;
use crate::manager::Component;


//------------ Tcp -----------------------------------------------------------

/// An RTR server atop unencrypted, plain TCP.
#[derive(Debug, Deserialize)]
pub struct Tcp {
    listen: Vec<SocketAddr>,
    unit: Link,
}

impl Tcp {
    /// Runs the target.
    pub async fn run(mut self, component: Component) -> Result<(), ExitError> {
        let mut notify = NotifySender::new();
        let target = Source::default();
        for &addr in &self.listen {
            self.spawn_listener(addr, target.clone(), notify.clone())?;
        }

        loop {
            if let Ok(update) = self.unit.query().await {
                debug!(
                    "Target {}: Got update ({} entries)",
                    component.name(), update.set().len()
                );
                target.update(update);
                notify.notify()
            }
        }
    }

    /// Spawns a single listener onto the current runtime.
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
        if let Err(err) = listener.set_nonblocking(true) {
            error!(
                "Fatal: failed to set listener {} to non-blocking: {}.",
                addr, err
            );
            return Err(ExitError);
        }
        let listener = match TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(err) => {
                error!("Fatal error listening on {}: {}", addr, err);
                return Err(ExitError)
            }
        };
        tokio::spawn(async move {
            let listener = TcpListenerStream::new(listener);
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
    data: Arc<ArcSwap<SourceData>>,
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
                    current: Some(update.set().clone()),
                    diffs: Vec::new(),
                    timing: Timing::default(),
                }
            }
            Some(current) => {
                let diff = match update.get_usable_diff(data.unit_serial) {
                    Some(diff) => diff.clone(),
                    None => update.set().diff_from(current),
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
                        old_diff.extend(&diff).unwrap()
                    ))
                }
                let mut state = data.state;
                state.inc();
                SourceData {
                    state,
                    unit_serial: update.serial(),
                    current: Some(update.set().clone()),
                    diffs,
                    timing: Timing::default(),
                }
            }
        };

        self.data.store(new_data.into());
    }
}

impl PayloadSource for Source {
    type Set = payload::OwnedSetIter;
    type Diff = payload::OwnedDiffIter;

    fn ready(&self) -> bool {
        self.data.load().current.is_some()
    }

    fn notify(&self) -> State {
        self.data.load().state
    }

    fn full(&self) -> (State, Self::Set) {
        let this = self.data.load();
        match this.current.as_ref() {
            Some(current) => (this.state, current.owned_iter()),
            None => (this.state, payload::Set::default().owned_iter()),
        }
    }

    fn diff(&self, state: State) -> Option<(State, Self::Diff)> {
        let this = self.data.load();
        if this.current.is_none() || state.session() != this.state.session() {
            return None
        }

        this.get_diff(state.serial()).map(|diff| {
            (this.state, diff)
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
    current: Option<payload::Set>,

    /// The diffs we currently keep.
    ///
    /// The diff with the largest serial is first.
    diffs: Vec<(Serial, payload::Diff)>,

    /// The timing paramters for this source.
    timing: Timing,
}

impl SourceData {
    fn get_diff(&self, serial: Serial) -> Option<payload::OwnedDiffIter> {
        if serial == self.state.serial() {
            Some(payload::Diff::default().into_owned_iter())
        }
        else {
            self.diffs.iter().find_map(|item| {
                if item.0 == serial {
                    Some(item.1.owned_iter())
                }
                else {
                    None
                }
            })
        }
    }
}

