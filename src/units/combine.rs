//! Units that combine the updates from other units.

use std::sync::Arc;
use crossbeam_utils::atomic::AtomicCell;
use futures::future::{select, select_all, Either, FutureExt};
use rand::{thread_rng, Rng};
use serde::Deserialize;
use crate::metrics;
use crate::metrics::{Metric, MetricType, MetricUnit};
use crate::comms::{Gate, GateMetrics, Link, Terminated, UnitStatus};
use crate::manager::Component;


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

        let mut curr_idx: Option<usize> = None;

        // Outer loop picks a new source.
        loop {
            curr_idx = match self.pick(curr_idx) {
                Ok(curr_idx) => curr_idx,
                Err(_) => {
                    gate.update_status(UnitStatus::Gone).await;
                    return Err(Terminated)
                }
            };
            metrics.current_index.store(curr_idx);
            match curr_idx {
                Some(idx) => {
                    gate.update_status(UnitStatus::Healthy).await;
                    if let Some(update) = self.sources[idx].get_data() {
                        gate.update_data(update.clone()).await;
                    }
                }
                None => {
                    gate.update_status(UnitStatus::Stalled).await;
                }
            }

            // Inner loop works the source until it stalls
            loop {
                let (res, idx, _) = {
                    let res = select(
                        select_all(
                            self.sources.iter_mut().map(|link|
                                link.query().boxed()
                            )
                        ),
                        gate.process().boxed()
                    ).await;

                    match res {
                        // The select_all
                        Either::Left((res, _)) => { res }

                        // The gate.process
                        Either::Right(_) => continue,
                    }
                };

                match res {
                    Ok(update) => {
                        if Some(idx) == curr_idx {
                            gate.update_data(update.clone()).await;
                        }
                    }
                    Err(UnitStatus::Stalled) => {
                        if Some(idx) == curr_idx {
                            break
                        }
                    }
                    Err(UnitStatus::Healthy) => {
                        if curr_idx.is_none() {
                            break
                        }
                    }
                    _ => ()
                }
            }
        }
    }

    /// Pick the next healthy source.
    ///
    /// This will return `None`, if no healthy source is currently available.
    /// It will error out if all sources have gone.
    fn pick(&self, curr: Option<usize>) -> Result<Option<usize>, Terminated> {
        // Here’s what we do in case of random picking: We only pick the next
        // source at random and then loop around. That’s not truly random but
        // deterministic.
        let mut next = if self.random {
            thread_rng().gen_range(0..self.sources.len())
        }
        else if let Some(curr) = curr {
            (curr + 1) % self.sources.len()
        }
        else {
            0
        };
        let mut only_gone = true;
        for _ in 0..self.sources.len() {
            match self.sources[next].get_status() {
                UnitStatus::Healthy => {
                    return Ok(Some(next))
                }
                UnitStatus::Stalled => {
                    only_gone = false;
                }
                UnitStatus::Gone => { }
            }
            next = (next + 1) % self.sources.len()
        }
        if only_gone {
            Err(Terminated)
        }
        else {
            Ok(None)
        }
    }
}


//------------ AnyMetrics ----------------------------------------------------

#[derive(Debug, Default)]
struct AnyMetrics {
    current_index: AtomicCell<Option<usize>>,
    gate: Arc<GateMetrics>,
}

impl AnyMetrics {
    const CURRENT_INDEX_METRIC: Metric = Metric::new(
        "current_index", "the index of the currenly selected source",
        MetricType::Gauge, MetricUnit::Info
    );
}

impl AnyMetrics {
    fn new(gate: &Gate) -> Self {
        AnyMetrics {
            current_index: Default::default(),
            gate: gate.metrics(),
        }
    }
}

impl metrics::Source for AnyMetrics {
    fn append(&self, unit_name: &str, target: &mut metrics::Target)  {
        target.append_simple(
            &Self::CURRENT_INDEX_METRIC, Some(unit_name),
            self.current_index.load().map(|v| v as isize).unwrap_or(-1)
        );
        self.gate.append(unit_name, target);
    }
}


//============ Tests =========================================================

#[cfg(test)]
mod test {
    use super::*;
    use futures::join;
    use tokio::runtime;
    use crate::{test, units};
    use crate::manager::Manager;
    use crate::payload::testrig;

    #[tokio::test(flavor = "multi_thread")]
    async fn wake_up_again() {
        use crate::comms::UnitStatus::*;

        let mut manager = Manager::new();

        let (u1, u2, u3, mut t) = manager.add_components(
            &runtime::Handle::current(),
            |units, targets| {
                let (u, u1c) = test::Unit::new();
                units.insert("u1", u);
                let (u, u2c) = test::Unit::new();
                units.insert("u2", u);
                let (u, u3c) = test::Unit::new();
                units.insert("u3", u);

                units.insert("any", units::Unit::Any(Any {
                    sources: vec!["u1".into(), "u2".into(), "u3".into()],
                    random: false
                }));

                let (t, tc) = test::Target::new("any");
                targets.insert("t", t);

                (u1c, u2c, u3c, tc)
            }
        ).unwrap();

        // Set all units to stalled, check that the target goes stalled.
        join!(u1.status(Stalled), u2.status(Stalled), u3.status(Stalled));
        assert_eq!(t.recv().await, Err(Stalled));

        // Set one unit to healthy.
        u2.data(testrig::update(&[1])).await;
        u2.status(Healthy).await;
        assert_eq!(t.recv().await, Err(Healthy));
        assert_eq!(t.recv().await, Ok(testrig::update(&[1])));

        // Set another unit to healthy. This shouldn’t change anything.
        u1.data(testrig::update(&[2])).await;
        u1.status(Healthy).await;

        // Stall them both again.
        join!(u1.status(Stalled), u2.status(Stalled));
        assert_eq!(t.recv().await, Err(Stalled));

        // Now unstall one again.
        u3.data(testrig::update(&[3])).await;
        u3.status(Healthy).await;
        assert_eq!(t.recv().await, Err(Healthy));
        assert_eq!(t.recv().await, Ok(testrig::update(&[3])));
    }
}

