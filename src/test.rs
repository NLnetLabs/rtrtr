//! Helpful things for testing.
#![cfg(test)]

use daemonbase::error::ExitError;
use tokio::sync::{mpsc, oneshot};
use crate::{payload, targets, units};
use crate::comms::{Gate, Link, Terminated, UnitUpdate};
use crate::manager::Component;


//------------ Unit ----------------------------------------------------------

/// A unit that only does what it is told.
#[derive(Debug)]
pub struct Unit {
    rx: mpsc::Receiver<(UnitUpdate, oneshot::Sender<()>)>,
}

impl Unit {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> (units::Unit, UnitController) {
        let (tx, rx) = mpsc::channel(10);
        (units::Unit::Test(Self { rx }), UnitController { tx })
    }

    pub async fn run(
        mut self, _component: Component, mut gate: Gate
    ) -> Result<(), Terminated> {
        while let Some((update, tx)) = gate.process_until(
            self.rx.recv()
        ).await? {
            gate.update(update).await;
            tx.send(()).unwrap();
        }
        Err(Terminated)
    }
}


//------------ UnitController ------------------------------------------------

/// A controller for telling the test unit what to do.
#[derive(Clone, Debug)]
pub struct UnitController {
    tx: mpsc::Sender<(UnitUpdate, oneshot::Sender<()>)>,
}

impl UnitController {
    pub async fn send_update(&self, update: UnitUpdate) {
        let (tx, rx) = oneshot::channel();
        self.tx.send((update, tx)).await.expect("unit was terminated");
        rx.await.unwrap()
    }

    pub async fn send_payload(&self, update: payload::Update) {
        self.send_update(UnitUpdate::Payload(update)).await
    }

    pub async fn send_stalled(&self) {
        self.send_update(UnitUpdate::Stalled).await
    }

    pub async fn send_gone(&self) {
        self.send_update(UnitUpdate::Gone).await
    }
}


//------------ Target --------------------------------------------------------

/// A target that allows checking what happened.
#[derive(Debug)]
pub struct Target {
    link: Link,
    tx: mpsc::UnboundedSender<UnitUpdate>,
}

impl Target {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(link: impl Into<Link>) -> (targets::Target, TargetController) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            targets::Target::Test(Self { link: link.into(), tx }),
            TargetController { rx }
        )
    }

    pub async fn run(
        mut self, _component: Component,
    ) -> Result<(), ExitError> {
        loop {
            self.tx.send(
                self.link.query().await
            ).expect("controller went away")
        }
    }
}


//------------ TargetController ----------------------------------------------

#[derive(Debug)]
pub struct TargetController {
    rx: mpsc::UnboundedReceiver<UnitUpdate>,
}

impl TargetController {
    pub fn recv_nothing(&mut self) -> Result<(), String> {
        use tokio::sync::mpsc::error::TryRecvError;

        match self.rx.try_recv() {
            Ok(update) => {
                Err(format!("expected no update, got {:?}", update))
            }
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => {
                Err("target disconnected".to_string())
            }
        }
    }

    pub async fn recv(&mut self) -> Result<UnitUpdate, String> {
        self.rx.recv().await.ok_or_else(|| "target was terminated".into())
    }

    pub async fn recv_payload(&mut self) -> Result<payload::Update, String> {
        match self.recv().await? {
            UnitUpdate::Payload(payload) => Ok(payload),
            other => Err(format!("expected payload, got {:?}", other)),
        }
    }

    pub async fn recv_stalled(&mut self) -> Result<(), String> {
        match self.recv().await? {
            UnitUpdate::Stalled => Ok(()),
            other => Err(format!("expected stalled status, got {:?}", other))
        }
    }
}


//------------ Helper Functions ----------------------------------------------

pub fn init_log() {
    stderrlog::new().verbosity(5).init().unwrap();
}


//============ Tests =========================================================

#[tokio::test(flavor = "multi_thread")]
async fn simple_comms() {
    use tokio::runtime;
    use crate::manager::Manager;
    use crate::payload::testrig;

    let mut manager = Manager::default();

    let (u, mut t) = manager.add_components(
        &runtime::Handle::current(),
        |units, targets| {
            let (u, uc) = Unit::new();
            units.insert("u", u);
            let (t, tc) = Target::new("u");
            targets.insert("t", t);

            (uc, tc)
        }
    ).unwrap();

    u.send_payload(testrig::update(&[2])).await;
    assert_eq!(t.recv_payload().await.unwrap(), testrig::update(&[2]));
}


