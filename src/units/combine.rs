/// Units that combine the updates from other units.

use std::sync::Arc;
use futures::future::{select_all, FutureExt};
use rand::{thread_rng, Rng};
use serde_derive::Deserialize;
use crate::metrics;
use crate::comms::{Gate, GateMetrics, Link, Terminated, UnitStatus};
use crate::manager::Component;
use crate::payload;


//------------ Any -----------------------------------------------------------

/// A unit selecting updates from one working unit from a set.
#[derive(Debug, Deserialize)]
pub struct Any {
    /// The set of units to choose from.
    sources: Vec<Link>,

    /// Whether to pick randomly from the sources.
    random: bool,
}

impl Any {
    pub async fn run(
        mut self, mut component: Component, mut gate: Gate
    ) -> Result<(), Terminated> {
        if self.sources.is_empty() {
            gate.update_status(UnitStatus::Gone).await;
            return Err(Terminated)
        }
        let metrics = Arc::new(AnyMetrics::new(&gate));
        component.register_metrics(metrics.clone());

        let mut gate = [gate];

        let mut curr_idx = self.pick(None);
        loop {
            let res = {
                select_all(
                    self.sources.iter_mut().enumerate().map(|(idx, link)| {
                        if idx == curr_idx {
                            AnySource::Active(link).run().boxed()
                        }
                        else {
                            AnySource::Suspended(link).run().boxed()
                        }
                    }).chain(gate.iter_mut().map(|gate| {
                        AnySource::Gate(gate).run().boxed()
                    }))
                ).await.0
            };
            match res {
                Ok(Some(update)) => {
                    gate[0].update_data(update).await
                }
                Ok(None) => {
                }
                Err(()) => {
                    self.sources[curr_idx].suspend().await;
                    curr_idx = self.pick(Some(curr_idx));
                }
            }
        }
    }


    fn pick(&self, curr: Option<usize>) -> usize {
        if self.random {
            thread_rng().gen_range(0, self.sources.len())
        }
        else if let Some(curr) = curr {
            (curr + 1) % self.sources.len()
        }
        else {
            0
        }
    }
}

enum AnySource<'a> {
    Active(&'a mut Link),
    Suspended(&'a mut Link),
    Gate(&'a mut Gate)
}

impl<'a> AnySource<'a> {
    async fn run(self) -> Result<Option<payload::Update>, ()> {
        match self {
            AnySource::Active(link) => {
                match link.query().await {
                    Ok(update) => Ok(Some(update)),
                    Err(UnitStatus::Healthy) => Ok(None),
                    Err(UnitStatus::Stalled) | Err(UnitStatus::Gone) => {
                        Err(())
                    }
                }
            }
            AnySource::Suspended(link) => {
                 let _  = link.query_suspended().await;
                 Ok(None)
            }
            AnySource::Gate(gate) => {
                let _ = gate.process().await;
                Ok(None)
            }
        }
    }
}


//------------ AnyMetrics ----------------------------------------------------

#[derive(Debug, Default)]
struct AnyMetrics {
    gate: Arc<GateMetrics>,
}

impl AnyMetrics {
    fn new(gate: &Gate) -> Self {
        AnyMetrics {
            gate: gate.metrics(),
        }
    }
}

impl metrics::Source for AnyMetrics {
    fn append(&self, unit_name: &str, target: &mut metrics::Target)  {
        self.gate.append(unit_name, target);
    }
}

