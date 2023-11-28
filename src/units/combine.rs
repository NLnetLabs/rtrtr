//! Units that combine the updates from other units.

use std::sync::Arc;
use crossbeam_utils::atomic::AtomicCell;
use futures::future::{select, select_all, Either, FutureExt};
use log::debug;
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
        self,
        mut component: Component,
        mut gate: Gate,
    ) -> Result<(), Terminated> {
        if self.sources.is_empty() {
            gate.update_status(UnitStatus::Gone).await;
            return Err(Terminated);
        }
        let metrics = Arc::new(AnyMetrics::new(&gate));
        component.register_metrics(metrics.clone());

        self.do_run(component.name().clone(), gate, metrics).await
    }

    async fn do_run(
        mut self,
        name: Arc<str>,
        mut gate: Gate,
        metrics: Arc<AnyMetrics>,
    ) -> Result<(), Terminated> {
        let mut curr_idx: Option<usize> = None;
        let mut updates: Vec<Option<payload::Update>> = vec![
            None; self.sources.len()
        ];

        // Outer loop picks a new source.
        loop {
            let new_idx = self.pick(curr_idx);
            if new_idx != curr_idx {
                match new_idx {
                    Some(idx) => {
                        debug!(
                            "Unit {}: switched source to index {}",
                            name, idx,
                        );
                        gate.update_status(UnitStatus::Healthy).await;
                    }
                    None => {
                        debug!(
                            "Unit {}: no active source",
                            name,
                        );
                        gate.update_status(UnitStatus::Stalled).await;
                    }
                }
                curr_idx = new_idx;
                metrics.current_index.store(new_idx);
            }
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
                        updates[idx] = Some(update.clone());
                        if Some(idx) == curr_idx {
                            gate.update_data(update.clone()).await;
                        }
                        else {
                            // We currently don’t have an active source but
                            // there was an update. Break to pick a new active
                            // source.
                            break
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn any_unit_should_propagate_status_changes() {
        // Create a channel that we can use to signal the upstream gate to
        // change status from Stalled to Healthy.
        let (status_changer_tx, mut status_changer_rx) =
            tokio::sync::mpsc::channel::<UnitStatus>(1);

        let (mut source_gate, mut source_gate_agent) = Gate::new();

        // Run the source unit Gate
        let _ = tokio::task::spawn(async move {
            loop {
                match source_gate
                    .process_until(status_changer_rx.recv())
                    .await
                {
                    Ok(Some(wanted_unit_status)) => {
                        eprintln!("Setting upstream unit status to: {wanted_unit_status}");
                        source_gate.update_status(wanted_unit_status).await;
                    }
                    Ok(None) => {
                        eprintln!(
                            "Upstream runner status changer stream ended."
                        );
                        break;
                    }
                    Err(Terminated) => todo!(),
                }
            }
        });

        // Create and run an Any unit connected upstream to the source Gate
        // and to be connected downstream to a target.
        let (any_gate, mut any_gate_agent) = Gate::new();
        tokio::task::spawn(async move {
            let source = source_gate_agent.create_link();

            let any = Any {
                sources: vec![source],
                random: false,
            };

            let metrics = Arc::new(AnyMetrics::default());
            any.do_run("mock".into(), any_gate, metrics).await.unwrap();
        });

        // Emulate a downstream target
        eprintln!("Connecting target to any unit");
        let mut downstream = any_gate_agent.create_link();

        // Note: we can't call query() here because it would block preventing
        // us from sending the status change to Healthy below, and we can't
        // run query() in a Tokio task because if by the time we send the
        // Healthy status change below the query() hasn't completed the
        // connect() send subscribe -> confirm subscribe dance the the status
        // change is not queued but dropped and thus never sent to the
        // downstream. We could sleep before sending the status Healthy change
        // but that is not deterministic and can make the test brittle due to
        // being time sensitive and thus dependent on the performance of the
        // test environment and CPU stealing, particularly a problem in shared
        // CI environments. Waiting a very long time to overcome such issues
        // is also not ideal as it would always make the test very slow. So
        // instead we use a test mode only fn called connect_only() that exits
        // once the connect handshake is complete but before blocking on
        // receiving updates via the connection.
        downstream.connect_only(false).await.unwrap();

        // Un-stall the upstream source.
        eprintln!("Requesting source to change status to Healthy");
        status_changer_tx.send(UnitStatus::Healthy).await.unwrap();

        // Expect the link to become healthy
        assert_status_change(&mut downstream, UnitStatus::Healthy).await;

        // Stall the upstream source again.
        eprintln!("Requesting source to change status to Stalled");
        status_changer_tx.send(UnitStatus::Stalled).await.unwrap();

        // Expect the link to stall
        assert_status_change(&mut downstream, UnitStatus::Stalled).await;
    }

    async fn assert_status_change(
        link: &mut Link,
        expected_status: UnitStatus
    ) {
        // Query the status of the Any gate via the downstream link.
        // We need some kind of timeout here otherwise the test will block
        // forever if the expected status change is never seen, but make it
        // reasonably high to avoid brittleness caused by test environment
        // differences. Unlike the delay we avoided having above, this
        // delay only slows down the test if the test is failing.
        const MAX_WAIT: Duration = Duration::from_secs(10);

        eprintln!("Wait for the Any unit to signal a status change");
        let timeout_res = tokio::time::timeout(MAX_WAIT, link.query()).await;
        assert!(!timeout_res.is_err(), "Timed out waiting for status change");

        // Confirm that the query result is an "error" (a status update)
        // We can't use assert_eq!() here as Update doesn't impl PartialEq/Eq.
        let query_res = timeout_res.unwrap();
        assert!(query_res.is_err());

        // Confirm that the gate indeed changed to status Healthy
        let new_status = query_res.unwrap_err();
        assert_eq!(new_status, expected_status);
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

