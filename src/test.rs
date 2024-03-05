//! Helpful things for testing.
#![cfg(test)]

use tokio::sync::mpsc;
use crate::{targets, units};
use crate::comms::{Gate, Link, Terminated, UnitStatus};
use crate::payload;
use crate::log::ExitError;
use crate::manager::Component;


//------------ Unit ----------------------------------------------------------

/// A unit that only does what it is told.
#[derive(Debug)]
pub struct Unit {
    rx: mpsc::Receiver<UnitCommand>,
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
        while let Some(cmd) = gate.process_until(self.rx.recv()).await? {
            match cmd {
                UnitCommand::Data(update) => {
                    gate.update_data(update).await
                }
                UnitCommand::Status(status) => {
                    gate.update_status(status).await
                }
            }
        }
        Err(Terminated)
    }
}


//------------ UnitController ------------------------------------------------

/// A controller for telling the test unit what to do.
#[derive(Clone, Debug)]
pub struct UnitController {
    tx: mpsc::Sender<UnitCommand>,
}

impl UnitController {
    pub async fn data(&self, data: payload::Update) {
        self.tx.send(
            UnitCommand::Data(data)
        ).await.expect("unit was terminated")
    }

    pub async fn status(&self, status: UnitStatus) {
        self.tx.send(
            UnitCommand::Status(status)
        ).await.expect("unit was terminated")
    }
}


//------------ UnitCommand ---------------------------------------------------

/// A command sent by the unit controller.
#[derive(Clone, Debug, Eq, PartialEq)]
enum UnitCommand {
    Data(payload::Update),
    Status(UnitStatus),
}


//------------ Target --------------------------------------------------------

/// A target that allows checking what happened.
#[derive(Debug)]
pub struct Target {
    link: Link,
    tx: mpsc::UnboundedSender<UnitCommand>,
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
            let cmd = match self.link.query().await {
                Ok(data) => UnitCommand::Data(data.clone()),
                Err(status) => UnitCommand::Status(status),
            };
            self.tx.send(cmd).expect("controller went away")
        }
    }
}


//------------ TargetController ----------------------------------------------

#[derive(Debug)]
pub struct TargetController {
    rx: mpsc::UnboundedReceiver<UnitCommand>,
}

impl TargetController {
    pub async fn recv(
        &mut self
    ) -> Result<payload::Update, UnitStatus> {
        match self.rx.recv().await.unwrap() {
            UnitCommand::Data(data) => Ok(data),
            UnitCommand::Status(status) => Err(status)
        }
    }
}


//============ Tests =========================================================

#[tokio::test(flavor = "multi_thread")]
async fn simple_comms() {
    use tokio::runtime;
    use crate::manager::Manager;
    use crate::payload::testrig;

    let mut manager = Manager::new();

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

    u.data(testrig::update(&[2])).await;
    assert_eq!(t.recv().await, Ok(testrig::update(&[2])));
}


