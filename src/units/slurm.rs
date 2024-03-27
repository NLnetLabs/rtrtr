//! Local Exceptions.

use std::{io, fs, thread};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime};
use arc_swap::ArcSwap;
use daemonbase::config::ConfigPath;
use log::debug;
use rpki::slurm::{SlurmFile, ValidationOutputFilters};
use serde::Deserialize;
use tokio::sync::Notify;
use crate::payload;
use crate::comms::{Gate, Link, Terminated, UnitUpdate};
use crate::manager::Component;


//------------ Configuration -------------------------------------------------

/// How long should the update thread sleep before checking files again?
const UPDATE_SLEEP: Duration = Duration::from_secs(2);


//------------ LocalExceptions -----------------------------------------------

/// A unit applying local exceptions from files.
#[derive(Debug, Deserialize)]
pub struct LocalExceptions {
    /// The source to read data from.
    source: Link,

    /// A list of paths to the SLURM files.
    files: Vec<ConfigPath>,
}

impl LocalExceptions {
    pub async fn run(
        mut self, mut component: Component, mut gate: Gate
    ) -> Result<(), Terminated> {
        component.register_metrics(gate.metrics());
        let files = ExceptionSet::new(
            self.files.into_iter().map(Into::into).collect()
        );

        // Whether we are ready to submit an update to our gate.
        //
        // This will stay at false until we have received our first
        // notification from the exception set indicating that it loaded all
        // files.
        let mut ready = false;
        loop {
            tokio::select! {
                biased;

                maybe_update = self.source.query() => {
                    match maybe_update {
                        UnitUpdate::Payload(_update) => { }
                        UnitUpdate::Gone => {
                            gate.update(UnitUpdate::Gone).await;
                            return Ok(())
                        }
                        _ => continue,
                    }
                }

                _ = files.notified() => {
                    ready = true;
                }

                _ = gate.process() => {
                    continue
                }
            }

            if let (true, Some(data)) = (ready, self.source.get_payload()) {
                gate.update(
                    UnitUpdate::Payload(files.apply(component.name(), data))
                ).await;
            }
        }
    }
}


//------------ ExceptionSet -------------------------------------------------

/// A collection of all the local exception files we are using.
struct ExceptionSet {
    data: Arc<ExceptionSetData>,

    /// An alive check for the update thread.
    ///
    /// If the set gets dropped, so does the arc and the thread can check on
    /// it via a weak reference to it.
    alive: Arc<()>,
}

impl ExceptionSet {
    fn new(paths: Vec<PathBuf>) -> Self {
        // Doing things in this order avoids the need for type annotations.
        let res = ExceptionSet {
            data: Arc::new(
                ExceptionSetData {
                    files: paths.iter().map(|_| Default::default()).collect(),
                    paths,
                    notify: Notify::new(),
                }
            ),
            alive: Arc::new(()),
        };
        let data = res.data.clone();
        let alive = Arc::downgrade(&res.alive);

        thread::spawn(move || {
            data.update_thread(alive)
        });

        res
    }

    fn apply(&self, unit: &str, update: &payload::Update) -> payload::Update {
        let mut set = update.set().clone();

        for (path, file) in
            self.data.paths.iter().zip(self.data.files.iter())
        {
            set = file.load().apply(unit, path, set);
            
        }

        payload::Update::new(set)
    }

    async fn notified(&self) {
        self.data.notify.notified().await
    }
}


//------------ ExceptionSetData ---------------------------------------------

struct ExceptionSetData {
    /// The paths to the various files.
    paths: Vec<PathBuf>,

    /// The content of the various files.
    ///
    /// This lives behind an `ArcSwap` so we can cheaply swap out the content
    /// if a file updates.
    files: Vec<ArcSwap<Content>>,

    /// A notifier for when the set has changed.
    notify: Notify,
}

impl ExceptionSetData {
    fn update_thread(self: Arc<Self>, alive: Weak<()>) {
        let mut modified = vec![None::<SystemTime>; self.paths.len()];

        loop {
            if alive.upgrade().is_none() {
                // The set has gone and so should we.
                return
            }

            let mut updated = false;

            for (path, (modified, content)) in
                self.paths.iter().zip(
                    modified.iter_mut().zip(self.files.iter())
                )
            {
                // We simply ignore any errors for now.
                if let Ok(true) = Self::update_file(path, modified, content) {
                    updated = true;
                }
            }

            if updated {
                self.notify.notify_one();
            }

            thread::sleep(UPDATE_SLEEP);
        }
    }

    /// Updates the given file if it changed.
    ///
    /// Returns `Ok(true)` if the file was updated or `Ok(false)` if not.
    fn update_file(
        path: &Path,
        old_modified: &mut Option<SystemTime>,
        content: &ArcSwap<Content>
    ) -> Result<bool, io::Error> {
        let new_modified = fs::metadata(path)?.modified()?;
        if let Some(old_modified) = old_modified.as_ref() {
            if new_modified <= *old_modified {
                return Ok(false)
            }
        }

        content.store(Arc::new(
            SlurmFile::from_reader(
                io::BufReader::new(
                    fs::File::open(path)?
                )
            )?.into()
        ));
        *old_modified = Some(new_modified);
        debug!("Updated Slurm file {}", path.display());
        Ok(true)
    }
}


//------------ Content -------------------------------------------------------

/// The content of a SLURM file in slightly pre-processed form.
#[derive(Default)]
struct Content {
    filters: ValidationOutputFilters,
    assertions: payload::Pack,
}

impl Content {
    fn apply(
        &self, unit: &str, path: &Path, set: payload::Set
    ) -> payload::Set {
        // First filters, then assertions.
        let filtered = set.filter(|payload| {
            !self.filters.drop_payload(payload)
        });
        let filtered_len = filtered.len();
        let mut builder = filtered.to_builder();
        builder.insert_pack(self.assertions.clone());
        let res = builder.finalize();
        debug!(
            "Unit {}: file {}: added {}, removed {}.",
            unit, path.display(),
            res.len() - filtered_len,
            set.len() - filtered_len
        );
        res
    }
}

impl From<SlurmFile> for Content {
    fn from(slurm: SlurmFile) -> Content {
        let mut assertions = payload::PackBuilder::empty();
        for payload in slurm.assertions.iter_payload() {
            assertions.insert_unchecked(payload)
        }
        let assertions = assertions.finalize();
        Content {
            filters: slurm.filters,
            assertions
        }
    }
}


//============ Tests =========================================================

#[cfg(test)]
mod test {
    use super::*;
    use crate::payload::testrig;
    use rpki::slurm::PrefixFilter;
    use rpki::rtr::payload::Payload;

    #[test]
    fn apply_content() {
        use rand::Rng;
        
        fn random_pack<T: Rng>(rng: &mut T, len: usize) -> payload::Pack {
            let mut res = payload::PackBuilder::empty();
            for _ in 0..len {
                res.insert_unchecked(testrig::p(rng.gen()))
            }
            res.finalize()
        }

        let mut rng = rand_pcg::Pcg32::new(
            0xcafef00dd15ea5e5, 0xa02bdbf7bb3c0a7
        );

        // First, let’s make a data set.
        let s1 = payload::Set::from(random_pack(&mut rng, 100));

        // Now, a pack that we first insert and then remove again via
        // local exceptions.
        let s2 = payload::Set::from(random_pack(&mut rng, 10));

        // And a pack which is what we are going to insert via local
        // exceptions.
        let p3 = random_pack(&mut rng, 15);

        // Now we can make the input and output sets.
        let input = s1.merge(&s2);
        let output = s1.merge(&payload::Set::from(p3.clone()));

        // Time to make the content.
        let content = Content {
            filters: ValidationOutputFilters {
                prefix: s2.iter().filter_map(|payload| {
                    match payload {
                        Payload::Origin(origin) => {
                            Some(PrefixFilter::new(
                                Some(origin.prefix.prefix()),
                                Some(origin.asn),
                                None
                            ))
                        }
                        _ => None
                    }
                }).collect(),
                bgpsec: Vec::new()
            },
            assertions: p3
        };

        assert_eq!(content.apply("none", Path::new("/"), input), output);
    }
}

