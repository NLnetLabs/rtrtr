//! The manager controlling the entire operation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use log::error;
use serde_derive::Deserialize;
use tokio::runtime::Runtime;
use crate::metrics;
use crate::comms::{Gate, GateAgent, Link};
use crate::config::{Config, ConfigFile, Marked};
use crate::log::Failed;
use crate::targets::Target;
use crate::units::Unit;


//------------ Component -----------------------------------------------------

/// A component.
///
/// Upon being started, every component receives one of these. It can use it
/// to communicate with the manager.
#[derive(Debug)]
pub struct Component {
    name: Arc<str>,
    metrics: metrics::Collection,
}

impl Component {
    fn new(name: String, metrics: metrics::Collection) -> Self {
        Component { name: name.into(), metrics }
    }

    /// Returns the name of the component.
    pub fn name(&self) -> &Arc<str> {
        &self.name
    }

    /// Register a metrics source.
    pub fn register_metrics(&mut self, source: Arc<dyn metrics::Source>) {
        self.metrics.register(self.name.clone(), Arc::downgrade(&source));
    }
}


//------------ Manager -------------------------------------------------------

#[derive(Default)]
pub struct Manager {
    units: HashMap<String, GateAgent>,

    pending: HashMap<String, Gate>,

    metrics: metrics::Collection,
}


impl Manager {
    pub fn new() -> Self {
        Default::default()
    }

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
        let config = match Config::from_toml(file.bytes()) {
            Ok(config) => config,
            Err(err) => {
                error!("{}: {}", file.path(), err);
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

    /// Spawns all units and targets in the config unto the given runtime.
    ///
    /// # Panics
    ///
    /// The method panics if the config hasn’t been successfully loaded via
    /// the same manager earlier.
    pub fn spawn(&mut self, config: &mut Config, runtime: &Runtime) {
        for (name, unit) in config.units.units.drain() {
            let gate = self.pending.remove(&name).unwrap();
            let controller = Component::new(name, self.metrics.clone());
            runtime.spawn(unit.run(controller, gate));
        }

        for (name, target) in config.targets.targets.drain() {
            runtime.spawn(target.run(name));
        }
    }

    pub fn metrics(&self) -> metrics::Collection {
        self.metrics.clone()
    }
}


//------------ UnitSet -------------------------------------------------------

#[derive(Deserialize)]
#[serde(transparent)]
pub struct UnitSet {
    units: HashMap<String, Unit>,
}


//------------ TargetSet -----------------------------------------------------

#[derive(Default, Deserialize)]
#[serde(transparent)]
pub struct TargetSet {
    targets: HashMap<String, Target>,
}

impl TargetSet {
    pub fn new() -> Self {
        Default::default()
    }
}


//------------ LoadUnit ------------------------------------------------------

struct LoadUnit {
    gate: Option<Gate>,
    agent: GateAgent,
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


thread_local!(
    static GATES: RefCell<Option<HashMap<String, LoadUnit>>> =
        RefCell::new(None)
);


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

