//! Communication between components.
//!
//! The main purpose of communication is for a unit to announce updates to
//! its data set and operational state to all other components that are
//! interested. It also takes care of managing these communication lines.
//!
//! There are two three main types here: Each unit has a single [`Gate`] to
//! which it hands its updates. The opposite end is called a [`Link`] and
//! by any interested component. A [`GateAgent`] is a reference to a gate
//! that can be used to create new links.
//!
//! The type [`GateMetrics`] can be used by units to provide some obvious
//! metrics such as the number of payload units in the data set or the time
//! of last update based on the updates sent to the gate.

use std::fmt;
use std::sync::atomic;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize};
use chrono::{DateTime, Utc};
use crossbeam_utils::atomic::AtomicCell;
use slab::Slab;
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use crate::{manager, metrics, payload};
use crate::config::Marked;
use crate::metrics::{Metric, MetricType, MetricUnit};


//------------ Configuration -------------------------------------------------

/// The queue length of an update channel.
const UPDATE_QUEUE_LEN: usize = 8;

/// The queue length of a command channel.
const COMMAND_QUEUE_LEN: usize = 16;


//------------ Gate ----------------------------------------------------------

/// A communication gate representing the upstream entrance.
///
/// Each unit receives exactly one gate. Whenever it has new data or its
/// status changes, it sends these to (through?) the gate which takes care
/// of distributing the information to whoever is interested.
///
/// A gate may be active or dormant. It is active if there is at least one
/// party interested in receiving data updates. Otherwise it is dormant.
/// Obviously, there is no need for a unit with a dormant gate to produce
/// any updates. Units are, in fact, encouraged to suspend their operation
/// until their gate becomes active again.
///
/// In order for the gate to maintain its own state, the unit needs to
/// regularly run the [`process`](Self::process) method. In return,
/// the unit will receive an update to the gate’s state as soon as it
/// becomes available.
///
/// Sending of updates happens via the [`update_data`](Self::update_data) and
/// [`update_status`](Self::update_status) methods.
#[derive(Debug)]
pub struct Gate {
    /// Receiver for commands sent in by the links.
    commands: mpsc::Receiver<GateCommand>,

    /// Senders to all links.
    updates: Slab<UpdateSender>,

    /// Number of suspended senders.
    suspended: usize,

    /// The current unit status.
    unit_status: UnitStatus,

    /// The gate metrics.
    metrics: Arc<GateMetrics>,
}


impl Gate {
    /// Creates a new gate.
    ///
    /// The function returns a gate and a gate agent that allows creating new
    /// links. Typically, you would pass the gate to a subsequently created
    /// unit and keep the agent around for future use.
    pub fn new() -> (Gate, GateAgent) {
        let (tx, rx) = mpsc::channel(COMMAND_QUEUE_LEN);
        let gate = Gate {
            commands: rx,
            updates: Slab::new(),
            suspended: 0,
            unit_status: UnitStatus::default(),
            metrics: Default::default(),
        };
        let agent = GateAgent { commands: tx };
        (gate, agent)
    }

    /// Returns a reference to the gate metrics.
    pub fn metrics(&self) -> Arc<GateMetrics> {
        self.metrics.clone()
    }

    /// Runs the gate’s internal machine.
    ///
    /// This method returns a future that runs the gate’s internal machine.
    /// It resolves once the gate’s status changes. It can be dropped at any
    /// time. In this case, the gate will pick up where it left off when the
    /// method is called again.
    ///
    /// The method will resolve into an error if the unit should terminate.
    /// This is the case if all links and gate agents refering to the gate
    /// have been dropped.
    pub async fn process(&mut self) -> Result<GateStatus, Terminated> {
        let status = self.get_gate_status();
        loop {
            let command = match self.commands.recv().await {
                Some(command) => command,
                None => return Err(Terminated)
            };

            match command {
                GateCommand::Suspension { slot, suspend } => {
                    self.suspension(slot, suspend)
                }
                GateCommand::Subscribe { suspended, response } => {
                    self.subscribe(suspended, response)
                }
            }

            let new_status = self.get_gate_status();
            if new_status != status {
                return Ok(new_status)
            }
        }
    }

    /// Updates the data set of the unit.
    ///
    /// This method will send out the update to all active links. It will
    /// also update the gate metrics based on the update.
    pub async fn update_data(&mut self, update: payload::Update) {
        for (_, item) in &mut self.updates {
            if item.suspended {
                continue
            }
            match item.sender.as_mut() {
                Some(sender) => {
                    if sender.send(Ok(update.clone())).await.is_ok() {
                        continue
                    }
                }
                None => continue
            }
            item.sender = None
        }
        self.updates.retain(|_, item| item.sender.is_some());
        self.metrics.update(&update);
    }

    /// Updates the unit status.
    ///
    /// The method sends out the new status to all links.
    pub async fn update_status(&mut self, update: UnitStatus) {
        self.unit_status = update;
        for (_, item) in &mut self.updates {
            match item.sender.as_mut() {
                Some(sender) => {
                    if sender.send(Err(update)).await.is_ok() {
                        continue
                    }
                }
                None => continue
            }
            item.sender = None
        }
        self.updates.retain(|_, item| item.sender.is_some());
        self.metrics.update_status(update);
    }

    /// Returns the current gate status.
    pub fn get_gate_status(&self) -> GateStatus {
        if self.suspended == self.updates.len() {
            GateStatus::Dormant
        }
        else {
            GateStatus::Active
        }
    }

    /// Processes a suspension command.
    fn suspension(&mut self, slot: usize, suspend: bool) {
        if let Some(item) = self.updates.get_mut(slot) {
            item.suspended = suspend
        }
    }

    /// Processes a subscribe command.
    fn subscribe(
        &mut self,
        suspended: bool,
        response: oneshot::Sender<SubscribeResponse>
    ) {
        let (tx, receiver) = mpsc::channel(UPDATE_QUEUE_LEN);
        let slot = self.updates.insert(UpdateSender {
            sender: Some(tx),
            suspended,
        });
        let subscription = SubscribeResponse {
            slot,
            receiver,
            unit_status: self.unit_status
        };
        if let Err(subscription) = response.send(subscription) {
            self.updates.remove(subscription.slot);
        }
    }
}


//------------ GateAgent -----------------------------------------------------

/// A reprensentative of a gate allowing creation of new links for it.
///
/// The agent can be cloned and passed along. The method
/// [`create_link`](Self::create_link) can be used to create a new link.
///
/// Yes, the name is a bit of a mixed analogy.
#[derive(Clone, Debug)]
pub struct GateAgent {
    commands: mpsc::Sender<GateCommand>,
}

impl GateAgent {
    /// Creates a new link to the gate.
    pub fn create_link(&mut self) -> Link {
        Link::new(self.commands.clone())
    }
}


//------------ GateMetrics ---------------------------------------------------

/// Metrics about the updates distributed via the gate.
///
/// This type is a [`metrics::Source`](crate::metrics::Source) that provides a
/// number of metrics for a unit that can be derived from the updates sent by
/// the unit and thus are common to all units.
///
/// Gates provide access to values of this type via the [`Gate::metrics`]
/// method. They will be stored behind an arc and can be kept and passed
/// around freely.
#[derive(Debug, Default)]
pub struct GateMetrics {
    /// The current unit status.
    status: AtomicCell<UnitStatus>,

    /// The serial number of the last update.
    serial: AtomicU32,

    /// The number of payload items in the last update.
    count: AtomicUsize,

    /// The date and time of the last update.
    ///
    /// If there has never been an update, this will be `None`.
    update: AtomicCell<Option<DateTime<Utc>>>,
}

impl GateMetrics {
    /// Updates the metrics to match the given update.
    fn update(&self, update: &payload::Update) {
        self.serial.store(update.serial().into(), atomic::Ordering::Relaxed);
        self.count.store(update.set().len(), atomic::Ordering::Relaxed);
        self.update.store(Some(Utc::now()));
    }

    /// Updates the metrics to match the given unit status.
    fn update_status(&self, status: UnitStatus) {
        self.status.store(status)
    }
}

impl GateMetrics {
    const STATUS_METRIC: Metric = Metric::new(
        "unit_status", "the operational status of the unit",
        MetricType::Text, MetricUnit::Info
    );
    const SERIAL_METRIC: Metric = Metric::new(
        "gate_serial", "the serial number of the unit's updates",
        MetricType::Counter, MetricUnit::Info
    );
    const COUNT_METRIC: Metric = Metric::new(
        "vrps", "the number of VRPs in the last update",
        MetricType::Gauge, MetricUnit::Total
    );
    const UPDATE_METRIC: Metric = Metric::new(
        "last_update", "the date and time of the last update",
        MetricType::Text, MetricUnit::Info
    );
    const UPDATE_AGO_METRIC: Metric = Metric::new(
        "since_last_update", "the number of seconds since the last update",
        MetricType::Gauge, MetricUnit::Second
    );
}

impl metrics::Source for GateMetrics {
    /// Appends the current gate metrics to a target.
    ///
    /// The name of the unit these metrics are associated with is given via
    /// `unit_name`.
    fn append(&self, unit_name: &str, target: &mut metrics::Target)  {
        target.append_simple(
            &Self::STATUS_METRIC, Some(unit_name), self.status.load()
        );
        target.append_simple(
            &Self::SERIAL_METRIC, Some(unit_name), 
            self.serial.load(atomic::Ordering::Relaxed)
        );
        target.append_simple(
            &Self::COUNT_METRIC, Some(unit_name),
            self.count.load(atomic::Ordering::Relaxed)
        );
        match self.update.load() {
            Some(update) => {
                target.append_simple(
                    &Self::UPDATE_METRIC, Some(unit_name),
                    update
                );
                let ago = Utc::now().signed_duration_since(update);
                let ago = (ago.num_milliseconds() as f64) / 1000.;
                target.append_simple(
                    &Self::UPDATE_AGO_METRIC, Some(unit_name), ago
                );
            }
            None => {
                target.append_simple(
                    &Self::UPDATE_METRIC, Some(unit_name),
                    "N/A"
                );
                target.append_simple(
                    &Self::UPDATE_AGO_METRIC, Some(unit_name), -1
                );
            }
        }
    }
}


//------------ Link ----------------------------------------------------------

/// A link to another unit.
///
/// The link allows tracking of updates of that other unit. This happens via
/// the [`query`](Self::query) method. A link’s owner can signal that they
/// are currently not interested in receiving updates via the
/// [`suspend`](Self::suspend) method. This suspension will automatically be
/// lifted the next time `query` is called.
///
/// Links can be created from the name of the unit they should be linking to
/// via [manager::load_link](crate::manager::load_link). This function is
/// also called implicitly through the impls for `Deserialize` and `From`.
/// Note, however, that the function only adds the link to a list of links
/// to be properly connected by the manager later. 
#[derive(Debug, Deserialize)]
#[serde(from = "String")]
pub struct Link {
    /// A sender of commands to the gate.
    commands: mpsc::Sender<GateCommand>,

    /// The connection to the unit.
    ///
    /// If this is `None`, the link has not been connected yet.
    connection: Option<LinkConnection>,

    /// The current unit status.
    unit_status: UnitStatus,

    /// Are we currently suspended?
    suspended: bool,
}

#[derive(Debug)]
struct LinkConnection {
    /// The slot number at the gate.
    slot: usize,

    /// The update receiver.
    updates: UpdateReceiver,
}

impl Link {
    /// Creates a new, unconnected link.
    fn new(commands: mpsc::Sender<GateCommand>) -> Self {
        Link {
            commands,
            connection: None,
            unit_status: UnitStatus::Healthy,
            suspended: false,
        }
    }

    /// Query for the next update.
    ///
    /// The method returns a future that resolves into the next update. The
    /// future can be dropped safely at any time.
    ///
    /// The future either resolves into a payload update or the connected
    /// unit’s new status as the error variant.  The current status is also
    /// available via the `get_status` method.
    ///
    /// If the link is currently suspended, calling this method will lift the
    /// suspension.
    pub async fn query(&mut self) -> Result<payload::Update, UnitStatus> {
        self.connect(false).await?;
        let conn = self.connection.as_mut().unwrap();

        match conn.updates.recv().await {
            Some(Ok(update)) => Ok(update),
            Some(Err(status)) => {
                self.unit_status = status;
                Err(status)
            }
            None => {
                self.unit_status = UnitStatus::Gone;
                Err(UnitStatus::Gone)
            }
        }
    }

    /// Query a suspended link.
    ///
    /// When a link is suspended, it still received updates to the unit’s
    /// status. These updates can also be queried for explicitly via this
    /// method.
    ///
    /// Much like `query`, the future returned by this method can safely be
    /// dropped at any time.
    pub async fn query_suspended(&mut self) -> UnitStatus {
        if let Err(err) = self.connect(true).await {
            return err
        }
        let conn = self.connection.as_mut().unwrap();

        loop {
            match conn.updates.recv().await {
                Some(Ok(_)) => continue,
                Some(Err(status)) => return status,
                None => {
                    self.unit_status = UnitStatus::Gone;
                    return UnitStatus::Gone
                }
            }
        }
    }

    /// Suspends the link.
    ///
    /// A suspended link will not receive any payload updates from the
    /// connected unit. It will, however, still receive status updates.
    ///
    /// The suspension is lifted automatically the next time `query` is
    /// called.
    ///
    /// Note that this is an async method that needs to be awaited in order
    /// to do anything.
    pub async fn suspend(&mut self) {
        if !self.suspended {
            self.request_suspend(true).await
        }
    }

    /// Request suspension from the gate.
    async fn request_suspend(&mut self, suspend: bool) {
        if self.connection.is_none() {
            return
        }

        let conn = self.connection.as_mut().unwrap();
        if self.commands.send(GateCommand::Suspension {
            slot: conn.slot,
            suspend
        }).await.is_err() {
            self.unit_status = UnitStatus::Gone
        }
        else {
            self.suspended = suspend
        }
    }

    /// Returns the current status of the connected unit.
    pub fn get_status(&self) -> UnitStatus {
        self.unit_status
    }

    /// Connects the link to the gate.
    async fn connect(&mut self, suspended: bool) -> Result<(), UnitStatus> {
        if self.connection.is_some() {
            return Ok(())
        }
        if let UnitStatus::Gone = self.unit_status {
            return Err(UnitStatus::Gone)
        }

        let (tx, rx) = oneshot::channel();
        if self.commands.send(
            GateCommand::Subscribe { suspended, response: tx }
        ).await.is_err() {
            self.unit_status = UnitStatus::Gone;
            return Err(UnitStatus::Gone)
        }
        let sub = match rx.await {
            Ok(sub) => sub,
            Err(_) => {
                self.unit_status = UnitStatus::Gone;
                return Err(UnitStatus::Gone)
            }
        };
        self.connection = Some(LinkConnection {
            slot: sub.slot,
            updates: sub.receiver,
        });
        self.unit_status = sub.unit_status;
        self.suspended = suspended;
        if self.unit_status == UnitStatus::Gone {
            Err(UnitStatus::Gone)
        }
        else {
            Ok(())
        }
    }
}

impl From<Marked<String>> for Link {
    fn from(name: Marked<String>) -> Self {
        manager::load_link(name)
    }
}

impl From<String> for Link {
    fn from(name: String) -> Self {
        manager::load_link(name.into())
    }
}


//------------ GateStatus ----------------------------------------------------

/// The status of a gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateStatus {
    /// The gate is connected to at least one active link.
    ///
    /// The unit owning this gate should produce updates.
    Active,

    /// The gate is not connected to any active links.
    ///
    /// This doesn’t necessarily mean that there are no links at all, only
    /// that currently none of the links is interested in receiving updates
    /// from this unit.
    Dormant,
}

impl Default for GateStatus {
    fn default() -> GateStatus {
        GateStatus::Active
    }
}


//------------ UnitStatus ----------------------------------------------------

/// The operational status of a unit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnitStatus {
    /// The unit is ready to produce data updates.
    ///
    /// Note that this status does not necessarily mean that the unit is
    /// actually producing updates, only that it could. That is, if a unit’s
    /// gate is dormant and the unit ceases operation because nobody cares,
    /// it is still in healthy status.
    Healthy,

    /// The unit had to temporarily suspend operation.
    ///
    /// If it sets this status, the unit will try to become healthy again
    /// later. The status is typically used if a server has become
    /// unavailable.
    Stalled,

    /// The unit had to permanently suspend operation.
    ///
    /// This status indicates that the unit will not become healthy ever
    /// again. Links to the unit can safely be dropped.
    Gone,
}

impl Default for UnitStatus {
    fn default() -> Self {
        UnitStatus::Healthy
    }
}

impl fmt::Display for UnitStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match *self {
            UnitStatus::Healthy => "healthy",
            UnitStatus::Stalled => "stalled",
            UnitStatus::Gone => "gone",
        })
    }
}


//------------ Terminated ----------------------------------------------------

/// An error signalling that a unit has been terminated.
///
/// In response to this error, a unit’s run function should return.
#[derive(Clone, Copy, Debug)]
pub struct Terminated;


//------------ GateCommand ---------------------------------------------------

/// A command sent by a link to a gate.
#[derive(Debug)]
enum GateCommand {
    /// Change the suspension state of a link.
    Suspension {
        /// The slot number of the link to be manipulated.
        slot: usize,

        /// Suspend the link?
        suspend: bool,
    },

    /// Subscribe to the gate.
    Subscribe {
        /// Should the subscription start in suspended state?
        suspended: bool,

        /// The sender for the response.
        ///
        /// The response payload is the slot number of the subscription.
        response: oneshot::Sender<SubscribeResponse>,
    }
}


//------------ UpdateSender --------------------------------------------------

/// The gate side of sending updates.
#[derive(Debug)]
struct UpdateSender {
    /// The actual sender.
    ///
    /// This is an option to facilitate deleted dropped links. When sending
    /// fails, we swap this to `None` and then go over the slab again and
    /// drop anything that is `None`. We need to do this because
    /// `Slab::retain` isn’t async but `mpsc::Sender::send` is.
    sender: Option<mpsc::Sender<Result<payload::Update, UnitStatus>>>,

    /// Are we currently suspended?
    suspended: bool
}


//------------ UpdateReceiver ------------------------------------------------

/// The link side of receiving updates.
type UpdateReceiver = mpsc::Receiver<Result<payload::Update, UnitStatus>>;


//------------ SubscribeResponse ---------------------------------------------

/// The response to a subscribe request.
#[derive(Debug)]
struct SubscribeResponse {
    /// The slot number of this subscription in the gate.
    slot: usize,

    /// The update receiver for this subscription.
    receiver: UpdateReceiver,

    /// The current unit status.
    unit_status: UnitStatus,
}

