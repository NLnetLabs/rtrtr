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

        let mut curr_idx: Option<usize> = None;
        let mut updates: Vec<Option<payload::Update>> = vec![
            None; self.sources.len()
        ];

        // Outer loop picks a new source.
        loop {
            curr_idx = self.pick(curr_idx);
            metrics.current_index.store(curr_idx);
            if let Some(idx) = curr_idx {
                if let Some(ref update) = updates[idx] {
                    gate.update_data(update.clone()).await;
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
                        updates[idx] = Some(update.clone());
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
    fn pick(&self, curr: Option<usize>) -> Option<usize> {
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
        for _ in 0..self.sources.len() {
            if self.sources[next].get_status() == UnitStatus::Healthy {
                return Some(next)
            }
            next = (next + 1) % self.sources.len()
        }
        None
    }
}


/*
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
        let mut curr_idx = None;
        loop {
            // Pick the next healthy source, if there is one.
            curr_idx = self.pick(curr_idx);
            metrics.current_index.store(curr_idx);

            println!("current index: {:?}", curr_idx);

            match curr_idx {
                Some(curr_idx) => self.run_healthy(&mut gate, curr_idx).await,
                None => self.run_stalled(&mut gate).await
            }
        }
    }

    /// Collects updates from a healthy source until it stalls.
    async fn run_healthy(&mut self, gate: &mut [Gate], curr_idx: usize) {
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
                    // Update from the current source.
                    gate[0].update_data(update).await
                }
                Ok(None) => {
                    // Status change we don’t care about.
                }
                Err(()) => {
                    // Current source has stalled.
                    break;
                }
            }
        }
    }

    /// Waits for any source becoming healthy.
    async fn run_stalled(&mut self, gate: &mut [Gate]) {
        loop {
            let res = {
                select_all(
                    self.sources.iter_mut().map(|link| {
                        AnySource::Suspended(link).run_stalled().boxed()
                    }).chain(gate.iter_mut().map(|gate| {
                        AnySource::Gate(gate).run_stalled().boxed()
                    }))
                ).await.0
            };
            println!("Back to healthy: {}", res);
            if res {
                break
            }
        }
    }

    /// Pick the next healthy source.
    ///
    /// This will return `None`, if no healthy source is currently available.
    fn pick(&self, curr: Option<usize>) -> Option<usize> {
        // Here’s what we do in case of random picking: We only pick the next
        // source at random and then loop around. That’s not truly random but
        // deterministic.
        let mut next = if self.random {
            thread_rng().gen_range(0, self.sources.len())
        }
        else if let Some(curr) = curr {
            (curr + 1) % self.sources.len()
        }
        else {
            0
        };
        for _ in 0..self.sources.len() {
            if self.sources[next].get_status() == UnitStatus::Healthy {
                return Some(next)
            }
            next = (next + 1) % self.sources.len()
        }
        None
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

    async fn run_stalled(self) -> bool {
        match self {
            AnySource::Active(_) => unreachable!(),
            AnySource::Suspended(link) => {
                 matches!(link.query_suspended().await, UnitStatus::Healthy)
            }
            AnySource::Gate(gate) => {
                let _ = gate.process().await;
                false
            }
        }
    }
}
*/


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

