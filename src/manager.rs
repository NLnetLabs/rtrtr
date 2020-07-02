//! The manager controlling the entire operation.

use std::{error, fmt};
use std::cell::RefCell;
use std::collections::HashMap;
use serde_derive::Deserialize;
use tokio::runtime::Runtime;
use crate::comms::{Gate, GateAgent, Link};
use crate::config::Marked;
use crate::targets::Target;
use crate::units::Unit;


//------------ Manager -------------------------------------------------------

#[derive(Default)]
pub struct Manager {
    units: HashMap<String, GateAgent>,

    pending: Vec<(String, Unit, Gate)>,
}


impl Manager {
    pub fn load<Op, R, E>(&mut self, op: Op) -> Result<R, LoadError>
    where
        Op: FnOnce() -> Result<(UnitSet, R), E>,
        E: error::Error + 'static
    {
        GATES.with(|gates| {
            gates.replace(
                Some(self.units.iter().map(|(key, value)| {
                    (key.clone(), value.clone().into())
                }).collect())
            )
        });
        let op_res = op();
        let gates = GATES.with(|gates| gates.replace(None) ).unwrap();
        let mut errs = LoadError::default();
        let (mut units, res) = match op_res {
            Ok(some) => some,
            Err(err) => {
                errs.push(Box::new(err));
                return Err(errs)
            }
        };

        // All gates that don’t have a unit must appear in units or else
        // we have unresolved links.
        
        for (name, load) in &gates {
            if load.gate.is_some() {
                if !units.units.contains_key(name) {
                    for link in &load.links {
                        errs.push(Box::new(UnresolvedLink {
                            name: name.clone(),
                            link: link.clone()
                        }))
                    }
                }
            }
        }

        if !errs.is_empty() {
            return Err(errs)
        }

        for (name, load) in gates {
            if let Some(gate) = load.gate {
                let unit = units.units.remove(&name).unwrap();
                self.pending.push((name, unit, gate))
            }
        }

        Ok(res)
    }

    pub fn spawn_units(&mut self, runtime: &Runtime) {
        for (name, unit, gate) in self.pending.drain(..) {
            runtime.spawn(unit.run(name, gate));
        }
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

    pub fn spawn_all(self, runtime: &Runtime) {
        for (name, target) in self.targets {
            let _ = runtime.spawn(target.run(name));
        }
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


//------------ LoadError -----------------------------------------------------

#[derive(Debug, Default)]
pub struct LoadError {
    errors: Vec<Box<dyn error::Error>>,
}

impl LoadError {
    fn push(&mut self, err: Box<dyn error::Error>) {
        self.errors.push(err)
    }

    pub fn iter(&self) -> impl Iterator<Item=&dyn error::Error> {
        self.errors.iter().map(|err| err.as_ref())
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.errors.len()
    }
}


//------------ UnresolvedLink ------------------------------------------------

#[derive(Clone, Debug)]
pub struct UnresolvedLink {
    name: String,
    link: Marked<()>,
}

impl fmt::Display for UnresolvedLink {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.link.format_mark(f)?;
        write!(f, "reference to undefined unit {}", self.name)
    }
}

impl error::Error for UnresolvedLink { }

