//! Units that combine the updates from other units.

use std::sync::Arc;
use crossbeam_utils::atomic::AtomicCell;
use futures::future::{select, select_all, Either, FutureExt};
use log::debug;
use rand::{thread_rng, Rng};
use serde::Deserialize;
use crate::{metrics, payload};
use crate::metrics::{Metric, MetricType, MetricUnit};
use crate::comms::{
    Gate, GateMetrics, Link, Terminated, UnitHealth, UnitUpdate
};
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
            gate.update(UnitUpdate::Gone).await;
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
                    gate.update(UnitUpdate::Gone).await;
                    return Err(Terminated)
                }
            };
            debug!(
                "Unit {}: current index is now {:?}",
                component.name(), curr_idx
            );
            metrics.current_index.store(curr_idx);
            match curr_idx {
                Some(idx) => {
                    if let Some(update) = self.sources[idx].payload() {
                        gate.update(
                            UnitUpdate::Payload(update.clone())
                        ).await;
                    }
                    else {
                        // This shouldn’t really happen (pick should always
                        // pick a source with an update), but this is still
                        // safe.
                        gate.update(UnitUpdate::Stalled).await;
                    }
                }
                None => {
                    gate.update(UnitUpdate::Stalled).await;
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
                    UnitUpdate::Payload(payload) => {
                        // If it is from our active source, send it on.
                        // If we don’t have an active source, break out of
                        // the loop because we may now have one.
                        if Some(idx) == curr_idx {
                            gate.update(UnitUpdate::Payload(payload)).await;
                        }
                        else if curr_idx.is_none() {
                            break
                        }
                    }
                    UnitUpdate::Stalled | UnitUpdate::Gone => {
                        // If our active unit stalls or dies, break.
                        // Otherwise we can ignore it.
                        if Some(idx) == curr_idx {
                            break
                        }
                    }
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
            match self.sources[next].health() {
                UnitHealth::Healthy => {
                    if self.sources[next].payload().is_some() {
                        return Ok(Some(next))
                    }
                    only_gone = false;
                }
                UnitHealth::Stalled => {
                    only_gone = false;
                }
                UnitHealth::Gone => { }
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


//------------ Merge ---------------------------------------------------------

/// A unit merging the data sets of all upstream units.
#[derive(Debug, Deserialize)]
pub struct Merge {
    /// The set of units whose data set should be merged.
    sources: Vec<Link>,
}

impl Merge {
    pub async fn run(
        mut self, mut component: Component, mut gate: Gate
    ) -> Result<(), Terminated> {
        if self.sources.is_empty() {
            gate.update(UnitUpdate::Gone).await;
            return Err(Terminated)
        }
        let metrics = gate.metrics();
        component.register_metrics(metrics.clone());

        loop {
            {
                let res = select(
                    select_all(
                        self.sources.iter_mut().map(|link|
                            link.query().boxed()
                        )
                    ),
                    gate.process().boxed()
                ).await;

                if let Either::Right(_) = res {
                    continue
                }
            }

            let mut output = payload::Set::default();
            for source in self.sources.iter() {
                if matches!(source.health(), UnitHealth::Healthy) {
                    if let Some(update) = source.payload() {
                        output = output.merge(update.set())
                    }
                }
            }
            gate.update(
                UnitUpdate::Payload(payload::Update::new(output))
            ).await;
        }
    }
}


//============ Tests =========================================================

#[cfg(test)]
mod test {
    use super::*;
    use tokio::runtime;
    use crate::{test, units};
    use crate::manager::Manager;
    use crate::payload::testrig;

    #[tokio::test]
    async fn wake_up_again() {
        test::init_log();
        let mut manager = Manager::default();

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

        // Set one unit to stalled, this triggers picking a new source healthy
        // with data but there isn’t one, so we go stalled.
        u1.send_stalled().await;
        t.recv_stalled().await.unwrap();

        // Now stall the other ones. That shouldn’t change anything.
        u2.send_stalled().await;
        t.recv_nothing().unwrap();
        u3.send_stalled().await;
        t.recv_nothing().unwrap();

        // Set one unit to healthy by sending a data update. Check that
        // the target unstalls with an update.
        u1.send_payload(testrig::update([1])).await;
        assert_eq!(t.recv_payload().await.unwrap(), testrig::update([1]));

        // Set another unit to healthy. This shouldn’t change anything.
        u2.send_payload(testrig::update([2])).await;
        t.recv_nothing().unwrap();

        // Now stall the first one and check that we get an update with the
        // second’s data.
        u1.send_stalled().await;
        assert_eq!(t.recv_payload().await.unwrap(), testrig::update([2]));

        // Now stall the second one, too, and watch us stall.
        u2.send_stalled().await;
        t.recv_stalled().await.unwrap();

        // Now unstall the third one and receive its data.
        u3.send_payload(testrig::update([3])).await;
        assert_eq!(t.recv_payload().await.unwrap(), testrig::update([3]));
    }
}

