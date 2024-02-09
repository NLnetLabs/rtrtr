//! Controlling the entire operation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use log::error;
use serde::Deserialize;
use reqwest::blocking::Client as HttpClient;
use tokio::runtime;
use crate::{http, metrics};
use crate::comms::{Gate, GateAgent, Link};
use crate::config::{Config, ConfigFile, Marked};
use crate::log::Failed;
use crate::targets::Target;
use crate::units::Unit;


//------------ Component -----------------------------------------------------

/// Facilities available to all components.
///
/// Upon being started, every component receives one of these. It provides
/// access to information and services available to all components.
#[derive(Debug)]
pub struct Component {
    /// The component’s name.
    name: Arc<str>,

    /// An HTTP client.
    http_client: HttpClient,

    /// A reference to the metrics collection.
    metrics: metrics::Collection,

    /// A reference to the HTTP resources collection.
    http_resources: http::Resources,
}

impl Component {
    /// Creates a new component from its, well, components.
    fn new(
        name: String,
        http_client: HttpClient,
        metrics: metrics::Collection,
        http_resources: http::Resources,
    ) -> Self {
        Component {
            name: name.into(), http_client, metrics, http_resources,
        }
    }

    /// Returns the name of the component.
    pub fn name(&self) -> &Arc<str> {
        &self.name
    }

    /// Returns a reference to an HTTP Client.
    pub fn http_client(&self) -> &HttpClient {
        &self.http_client
    }

    /// Register a metrics source.
    pub fn register_metrics(&mut self, source: Arc<dyn metrics::Source>) {
        self.metrics.register(self.name.clone(), Arc::downgrade(&source));
    }

    /// Register an HTTP resources.
    pub fn register_http_resource(
        &mut self, process: Arc<dyn http::ProcessRequest>
    ) {
        self.http_resources.register(Arc::downgrade(&process))
    }
}


//------------ Manager -------------------------------------------------------

/// A manager for components and auxiliary services.
#[derive(Default)]
pub struct Manager {
    /// The currently active units represented by agents to their gates..
    units: HashMap<String, GateAgent>,

    /// Gates for newly loaded, not yet spawned units.
    pending: HashMap<String, Gate>,

    /// An HTTP client.
    http_client: HttpClient,

    /// The metrics collection maintained by this managers.
    metrics: metrics::Collection,

    /// The HTTP resources collection maintained by this manager.
    http_resources: http::Resources,
}


impl Manager {
    /// Creates a new manager.
    pub fn new() -> Self {
        Default::default()
    }

    /// Loads the given config file.
    ///
    /// Parses the given file as a TOML config file. All links to units
    /// referenced in the configuration are pre-connected.
    ///
    /// If there are any errors in the config file, they are logged as errors
    /// and a generic error is returned.
    ///
    /// If the method succeeds, you need to spawn all units and targets via
    /// the [`spawn`](Self::spawn) method.
    pub fn load(
        &mut self, file: ConfigFile
    ) -> Result<Config, Failed> {
        // Prepare the thread-local used to allow serde load the links in the
        // units and targets.
        GATES.with(|gates| {
            gates.replace(
                Some(self.units.iter().map(|(key, value)| {
                    (key.clone(), value.clone().into())
                }).collect())
            )
        });

        // Now load the config file.
        let config = match Config::from_toml(file.bytes(), file.dir()) {
            Ok(config) => config,
            Err(err) => {
                match file.path() {
                    Some(path) => error!("{}: {}", path.display(), err),
                    None => error!("{}", err)
                }
                return Err(Failed)
            }
        };

        // All entries in the thread-local that have a gate are new. They must
        // appear in config’s units or we have unresolved links.
        let gates = GATES.with(|gates| gates.replace(None) ).unwrap();
        let mut errs = Vec::new();
        for (name, load) in gates {
            if let Some(gate) = load.gate {
                if !config.units.units.contains_key(&name) {
                    for mut link in load.links {
                        link.resolve_config(&file);
                        errs.push(link.mark(
                            format!("unresolved link to unit '{}'", name)
                        ))
                    }
                }
                else {
                    self.pending.insert(name, gate);
                }
            }
        }
        if !errs.is_empty() {
            for err in errs {
                error!("{}", err);
            }
            return Err(Failed)
        }

        Ok(config)
    }

    /// Allows creating units and adding them to the manager.
    ///
    /// Because creating units that contain links requires some setup work,
    /// this has to happen inside a closure. This closure should return a
    /// list of units 
    pub fn add_units<F, T>(
        &mut self, runtime: &runtime::Handle, op: F
    ) -> Result<T, Failed>
    where
        F: FnOnce(&mut UnitSet, &mut TargetSet) -> T
    {
        GATES.with(|gates| {
            gates.replace(
                Some(self.units.iter().map(|(key, value)| {
                    (key.clone(), value.clone().into())
                }).collect())
            )
        });
        
        let mut units = UnitSet::new();
        let mut targets = TargetSet::new();
        let res = op(&mut units, &mut targets);

        // All entries in the thread-local that have a gate are new. They must
        // appear in config’s units or we have unresolved links.
        let gates = GATES.with(|gates| gates.replace(None)).unwrap();
        let mut errs = Vec::new();
        for (name, load) in gates {
            if let Some(gate) = load.gate {
                if !units.units.contains_key(&name) {
                    errs.push(
                        format!("unresolved link to unit '{}'", name)
                    )
                }
                else {
                    self.pending.insert(name, gate);
                }
            }
        }
        if !errs.is_empty() {
            for err in errs {
                error!("{}", err);
            }
            return Err(Failed)
        }

        self.spawn(&mut units, &mut targets, runtime);
        Ok(res)
    }

    /// Spawns all units and targets unto the given runtime.
    ///
    /// # Panics
    ///
    /// The method panics if the config hasn’t been successfully loaded via
    /// the same manager earlier.
    pub fn spawn(
        &mut self,
        units: &mut UnitSet,
        targets: &mut TargetSet,
        runtime: &runtime::Handle,
    ) {
        for (name, unit) in units.units.drain() {
            let gate = match self.pending.remove(&name) {
                Some(gate) => gate,
                None => {
                    error!("Unit {} is unused and will not be started.", name);
                    continue
                }
            };
            let controller = Component::new(
                name, self.http_client.clone(), self.metrics.clone(),
                self.http_resources.clone()
            );
            runtime.spawn(unit.run(controller, gate));
        }

        for (name, target) in targets.targets.drain() {
            let controller = Component::new(
                name, self.http_client.clone(), self.metrics.clone(),
                self.http_resources.clone()
            );
            runtime.spawn(target.run(controller));
        }
    }

    /// Returns a new reference to the manager’s metrics collection.
    pub fn metrics(&self) -> metrics::Collection {
        self.metrics.clone()
    }

    /// Returns a new reference the the HTTP resources collection.
    pub fn http_resources(&self) -> http::Resources {
        self.http_resources.clone()
    }
}


//------------ UnitSet -------------------------------------------------------

/// A set of units to be started.
#[derive(Default, Deserialize)]
#[serde(transparent)]
pub struct UnitSet {
    units: HashMap<String, Unit>,
}

impl UnitSet {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, unit: Unit) {
        self.units.insert(name.into(), unit);
    }
}


//------------ TargetSet -----------------------------------------------------

/// A set of targets to be started.
#[derive(Default, Deserialize)]
#[serde(transparent)]
pub struct TargetSet {
    targets: HashMap<String, Target>,
}

impl TargetSet {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, target: Target) {
        self.targets.insert(name.into(), target);
    }
}

//------------ LoadUnit ------------------------------------------------------

/// A unit referenced during loading.
struct LoadUnit {
    /// The gate of the unit.
    ///
    /// This is some only if the unit is newly created and has not yet been
    /// spawned onto a runtime.
    gate: Option<Gate>,

    /// A gate agent for the unit.
    agent: GateAgent,

    /// A list of location of links in the config.
    ///
    /// This is only used for generating errors if non-existing units are
    /// referenced in the config file.
    links: Vec<Marked<()>>,
}

impl Default for LoadUnit {
    fn default() -> Self {
        let (gate, agent) = Gate::new();
        LoadUnit {
            gate: Some(gate),
            agent,
            links: Vec::new()
        }
    }
}

impl From<GateAgent> for LoadUnit {
    fn from(agent: GateAgent) -> Self {
        LoadUnit {
            gate: None,
            agent,
            links: Vec::new()
        }
    }
}


//------------ Loading Links -------------------------------------------------

thread_local!(
    static GATES: RefCell<Option<HashMap<String, LoadUnit>>> =
        RefCell::new(None)
);


/// Loads a link with the given name.
///
/// # Panics
///
/// This funtion panics if it is called outside of a run of
/// [`Manager::load`].
pub fn load_link(name: Marked<String>) -> Link {
    GATES.with(|gates| {
        let mut gates = gates.borrow_mut();
        let gates = gates.as_mut().unwrap();

        let mark = name.mark(());
        let name = name.into_inner();
        let unit = gates.entry(name).or_default();
        unit.links.push(mark);
        unit.agent.create_link()
    })
}

