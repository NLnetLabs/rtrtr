//! Communication between components.
//!
//! The main purpose of communication is for a unit is to announce updates to
//! its data set and operational state to all other components that are
//! interested. It also takes care of managing these communication lines.
//!
//! There are three main types here: Each unit has a single [`Gate`] to
//! which it hands its updates. The opposite end is called a [`Link`] and
//! is held by any interested component. A [`GateAgent`] is a reference to a
//! gate that can be used to create new links.
//!
//! The type [`GateMetrics`] can be used by units to provide some obvious
//! metrics such as the number of payload units in the data set or the time
//! of last update based on the updates sent to the gate.

use std::fmt;
use std::sync::atomic;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use chrono::{DateTime, Utc};
use crossbeam_utils::atomic::AtomicCell;
use futures_util::pin_mut;
use futures_util::future::{pending, select, Either, Future};
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

/// A communication gate representing the source of data.
///
/// Each unit receives exactly one gate. Whenever it has new data or its
/// status changes, it sends these to (through?) the gate which takes care
/// of distributing the information to whomever is interested.
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
            unit_status: Default::default(),
            metrics: Default::default(),
        };
        let agent = GateAgent { commands: tx };
        (gate, agent)
    }

    /// Returns a shareable reference to the gate metrics.
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
    /// This is the case if all links and gate agents referring to the gate
    /// have been dropped.
    pub async fn process(&mut self) -> Result<GateStatus, Terminated> {
        let status = self.gate_status();
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

            let new_status = self.gate_status();
            if new_status != status {
                return Ok(new_status)
            }
        }
    }

    /// Runs the gate’s internal machine until a future resolves.
    ///
    /// Ignores any gate status changes.
    pub async fn process_until<Fut: Future>(
        &mut self,
        fut: Fut
    ) -> Result<Fut::Output, Terminated> {
        pin_mut!(fut);

        loop {
            let process = self.process();
            pin_mut!(process);
            match select(process, fut).await {
                Either::Left((Err(_), _)) => return Err(Terminated),
                Either::Left((Ok(_), next_fut)) => {
                    fut = next_fut;
                }
                Either::Right((res, _)) => return Ok(res)
            }
        }
    }

    /// Updates the unit.
    ///
    /// This method will send out the update to all active links. It will
    /// also update the gate metrics based on the update.
    ///
    /// Returns whether the update changed the unit’s status.
    pub async fn update(&mut self, update: UnitUpdate) -> bool {
        if !self.unit_status.apply(&update) {
            return false
        }
        for (_, item) in &mut self.updates {
            if item.suspended {
                continue
            }
            match item.sender.as_mut() {
                Some(sender) => {
                    if sender.send(update.clone()).await.is_ok() {
                        continue
                    }
                }
                None => continue
            }
            item.sender = None
        }
        self.updates.retain(|_, item| item.sender.is_some());
        self.metrics.update(&self.unit_status);
        true
    }

    /// Returns the current gate status.
    pub fn gate_status(&self) -> GateStatus {
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
            unit_status: self.unit_status.clone(),
        };
        if let Err(subscription) = response.send(subscription) {
            self.updates.remove(subscription.slot);
        }
    }
}


//------------ GateAgent -----------------------------------------------------

/// A representative of a gate allowing creation of new links for it.
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
/// method. When stored behind an arc t can be kept and passed around freely.
#[derive(Debug, Default)]
pub struct GateMetrics {
    /// The current unit status.
    health: AtomicCell<UnitHealth>,

    /// The number of payload items in the last update.
    count: AtomicUsize,

    /// The date and time of the last update.
    ///
    /// If there has never been an update, this will be `None`.
    update: AtomicCell<Option<DateTime<Utc>>>,
}

impl GateMetrics {
    /// Updates the metrics to match the given update.
    fn update(&self, status: &UnitStatus) {
        if let Some(payload) = status.payload.as_ref() {
            self.count.store(
                payload.set().len(), atomic::Ordering::Relaxed
            );
        }
        self.update.store(Some(Utc::now()));
        self.health.store(status.health)
    }
}

impl GateMetrics {
    const STATUS_METRIC: Metric = Metric::new(
        "unit_status", "the operational status of the unit",
        MetricType::Text, MetricUnit::Info
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
            &Self::STATUS_METRIC, Some(unit_name), self.health.load()
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
    connection: ConnectionStatus,

    /// The current unit health.
    unit_status: UnitStatus,

    /// Are we currently suspended?
    suspended: bool,
}

#[derive(Debug)]
enum ConnectionStatus {
    /// The link is still unconnected.
    Unconnected,

    /// The link is connected and ready to receive updates.
    Active(LinkConnection),

    /// The link’s unit has gone.
    Gone
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
            connection: ConnectionStatus::Unconnected,
            unit_status: Default::default(),
            suspended: false,
        }
    }

    /// Returns the current health of the connected unit.
    pub fn health(&self) -> UnitHealth {
        self.unit_status.health
    }

    /// Returns the last update if there was one.
    pub fn payload(&self) -> Option<&payload::Update> {
        self.unit_status.payload.as_ref()
    }

    /// Query for the next update.
    ///
    /// The method returns a future that resolves into the next update. The
    /// future can be dropped safely at any time.
    ///
    /// If this method is called when the unit status is “gone,” the future
    /// will never resolve.
    pub async fn query(&mut self) -> UnitUpdate {
        if self.connect().await {
            // A connection attempt has been made. The unit status now
            // represents the initial update. If there is one, return it.
            // Otherwise we need to wait for the next update event.
            if let Some(update) = self.unit_status.to_update() {
                return update
            }
        }

        let conn = match self.connection {
            ConnectionStatus::Active(ref mut conn) => conn,
            ConnectionStatus::Unconnected | ConnectionStatus::Gone => {
                return pending().await
            }
        };
        match conn.updates.recv().await {
            Some(update) => {
                self.unit_status.apply(&update);
                update
            }
            None => {
                self.connection = ConnectionStatus::Gone;
                self.unit_status.health = UnitHealth::Gone;
                UnitUpdate::Gone
            }
        }
    }

    /// Connects the link to the gate if necessary.
    ///
    /// Returns `true` if a connection attempt was made – independently of
    /// whether that was successful or not – or `false` otherwise.
    async fn connect(&mut self) -> bool {
        if !matches!(self.connection, ConnectionStatus::Unconnected) {
            return false
        }

        let (tx, rx) = oneshot::channel();
        if self.commands.send(
            GateCommand::Subscribe { suspended: self.suspended, response: tx }
        ).await.is_err() {
            self.connection = ConnectionStatus::Gone;
            self.unit_status.health = UnitHealth::Gone;
            return true
        }
        let sub = match rx.await {
            Ok(sub) => sub,
            Err(_) => {
                self.connection = ConnectionStatus::Gone;
                self.unit_status.health = UnitHealth::Gone;
                return true
            }
        };
        self.connection = ConnectionStatus::Active(LinkConnection {
            slot: sub.slot,
            updates: sub.receiver,
        });
        self.unit_status = sub.unit_status;
        true
    }

    /// Suspends the link.
    ///
    /// This is merely a notification to the gate that the owner of the link
    /// isn’t currently interested in updates. The gate will, however, still
    /// send updates if it produces any. The link thus still needs to be
    /// queried regularly or else the queue will fill up.
    ///
    /// Note that this is an async method that needs to be awaited in order
    /// to do anything.
    pub async fn suspend(&mut self, suspend: bool) {
        if self.suspended != suspend {
            let conn = match self.connection {
                ConnectionStatus::Active(ref mut conn) => conn,
                _ => return
            };
            if self.commands.send(GateCommand::Suspension {
                slot: conn.slot,
                suspend
            }).await.is_err() {
                self.unit_status.health = UnitHealth::Gone
            }
            else {
                self.suspended = suspend
            }
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

impl<'a> From<&'a str> for Link {
    fn from(name: &'a str) -> Self {
        Self::from(String::from(name))
    }
}


//------------ GateStatus ----------------------------------------------------

/// The status of a gate.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum GateStatus {
    /// The gate is connected to at least one active link.
    ///
    /// The unit owning this gate should produce updates.
    #[default]
    Active,

    /// The gate is not connected to any active links.
    ///
    /// This doesn’t necessarily mean that there are no links at all, only
    /// that currently none of the links is interested in receiving updates
    /// from this unit.
    Dormant,
}


//------------ UnitHealth ----------------------------------------------------

/// A unit’s self-perceived ability to produce updates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum UnitHealth {
    /// The unit is ready to produce data updates.
    ///
    /// Note that this status does not necessarily mean that the unit is
    /// actually producing updates, only that it could. That is, if a unit’s
    /// gate is dormant and the unit ceases operation because nobody cares,
    /// it is still in healthy status.
    #[default]
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

impl<'a> From<&'a UnitUpdate> for UnitHealth {
    fn from(update: &'a UnitUpdate) -> Self {
        match update {
            UnitUpdate::Payload(_) => Self::Healthy,
            UnitUpdate::Stalled => Self::Stalled,
            UnitUpdate::Gone => Self::Gone,
        }
    }
}

impl fmt::Display for UnitHealth {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match *self {
            UnitHealth::Healthy => "healthy",
            UnitHealth::Stalled => "stalled",
            UnitHealth::Gone => "gone",
        })
    }
}


//------------ UnitStatus ----------------------------------------------------

/// A combination of both a unit’s health and the last payload update.
///
/// This is a helper type that makes it easier to apply updates.
#[derive(Clone, Debug, Default)]
struct UnitStatus {
    /// The current health of the unit.
    health: UnitHealth,

    /// The last payload update if there ever was one.
    payload: Option<payload::Update>,
}

impl UnitStatus {
    /// Applies a unit update to the status.
    ///
    /// Returns whether the status has changed.
    fn apply(&mut self, update: &UnitUpdate) -> bool {
        match update {
            UnitUpdate::Payload(payload) => {
                if matches!(self.health, UnitHealth::Healthy)
                    && Some(payload) == self.payload.as_ref()
                {
                    false
                }
                else {
                    self.health = UnitHealth::Healthy;
                    self.payload = Some(payload.clone());
                    true
                }
            }
            UnitUpdate::Stalled => {
                if matches!(self.health, UnitHealth::Stalled) {
                    false
                }
                else {
                    self.health = UnitHealth::Stalled;
                    true
                }
            }
            UnitUpdate::Gone => {
                if matches!(self.health, UnitHealth::Gone) {
                    false
                }
                else {
                    self.health = UnitHealth::Gone;
                    true
                }
            }
        }
    }

    /// Returns an update corresponding with the current unit status.
    ///
    /// This may be `None` if the unit status indicates that there hasn’t
    /// been an update yet.
    fn to_update(&self) -> Option<UnitUpdate> {
        match self.health {
            UnitHealth::Healthy => {
                self.payload.as_ref().map(|payload| {
                    UnitUpdate::Payload(payload.clone())
                })
            }
            UnitHealth::Stalled => Some(UnitUpdate::Stalled),
            UnitHealth::Gone => Some(UnitUpdate::Gone),
        }
    }
}


//------------ UnitUpdate ----------------------------------------------------

/// An update to the unit.
#[derive(Clone, Debug)]
pub enum UnitUpdate {
    /// A new payload set has become available.
    ///
    /// This also implies a change to unit status “healthy” if that is not the
    /// current state.
    Payload(payload::Update),

    /// The unit status has changed to “stalled.”
    Stalled,

    /// The unit status has changed to “gone.”
    Gone
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
    sender: Option<mpsc::Sender<UnitUpdate>>,

    /// Are we currently suspended?
    suspended: bool
}


//------------ UpdateReceiver ------------------------------------------------

/// The link side of receiving updates.
type UpdateReceiver = mpsc::Receiver<UnitUpdate>;


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

