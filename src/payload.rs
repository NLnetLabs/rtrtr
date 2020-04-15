use std::io;
use std::cmp::Ordering;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use log::debug;
use rpki_rtr::client::{VrpUpdate, VrpTarget};
use rpki_rtr::payload::{Action, Payload, Timing};
use rpki_rtr::state::{Serial, State};
use rpki_rtr::server::{NotifySender, VrpSource};


//------------ Set -----------------------------------------------------------

/// A set of payload.
#[derive(Clone, Debug, Default)]
pub struct Set {
    /// The payload items.
    ///
    /// This vec is guaranteed to be ordered and not contain duplicated at all
    /// times.
    items: Vec<Payload>,
}

impl Set {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /*
    /// Removes all items in `set` from `self`.
    pub fn remove_set(&mut self, set: &Set) {
        let mut set: &[Payload] = set.items.as_ref();
        self.items.retain(|item| {
            match set.binary_search(item) {
                Ok(pos) => {
                    set = &set[pos..];
                    false
                }
                Err(pos) => {
                    set = &set[pos..];
                    true
                }
            }
        })
    }
    */

    /// Returns the diff to get from `other` to `self`.
    pub fn diff_from(&self, other: &Set) -> Diff {
        let mut diff = Vec::new();
        let mut source = other.items.as_slice();
        let mut target = self.items.as_slice();

        // Process items while there’s some left in both sets.
        while let (Some(source_item), Some(target_item)) = (
            source.first(), target.first()
        ) {
            match source_item.cmp(target_item) {
                Ordering::Less => {
                    diff.push((Action::Withdraw, *source_item));
                    skip_first(&mut source);
                }
                Ordering::Equal => {
                    skip_first(&mut source);
                    skip_first(&mut target);
                }
                Ordering::Greater => {
                    diff.push((Action::Announce, *target_item));
                    skip_first(&mut target);
                }
            }
        }

        // Now at least one set is empty so we can just withdraw anything
        // left in source and announce anything left in target. Only one of
        // those will happen.
        for &item in source {
            diff.push((Action::Withdraw, item))
        }
        for &item in target {
            diff.push((Action::Announce, item))
        }
        Diff { items: diff }
    }
}

impl From<SetBuilder> for Set {
    fn from(builder: SetBuilder) -> Self {
        builder.finalize()
    }
}


//------------ SetIter ------------------------------------------------

pub struct SetIter {
    set: Arc<Set>,
    pos: usize,
}

impl From<Arc<Set>> for SetIter {
    fn from(set: Arc<Set>) -> Self {
        SetIter {
            set,
            pos: 0
        }
    }
}

impl Iterator for SetIter {
    type Item = Payload;

    fn next(&mut self) -> Option<Self::Item> {
        match self.set.items.get(self.pos) {
            Some(res) => {
                self.pos += 1;
                Some(*res)
            }
            None => None,
        }
    }
}


//------------ SetBuilder ---------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct SetBuilder {
    items: Vec<Payload>,
}

impl SetBuilder {
    pub fn push(&mut self, payload: Payload) {
        self.items.push(payload)
    }

    /*
    pub fn push_set(&mut self, set: Set) {
        self.items.extend(&set.items)
    }
    */

    pub fn finalize_strict(mut self) -> Option<Set> {
        self.items.sort_unstable();
        for pair in self.items.windows(2) {
            if pair[0] == pair[1] {
                return None
            }
        }
        Some(Set { items: self.items })
    }

    pub fn finalize(mut self) -> Set {
        self.items.sort_unstable();
        self.items.dedup();
        Set { items: self.items }
    }
}

impl From<Set> for SetBuilder {
    fn from(set: Set) -> Self {
        SetBuilder {
            items: set.items
        }
    }
}


//------------ Diff ---------------------------------------------------

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Diff {
    /// The diff items.
    ///
    /// This vec is guaranteed to be ordered by payload and will only ever
    /// contain at most one element for each payload.
    items: Vec<(Action, Payload)>,
}

impl Diff {
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn shared_iter(self: &Arc<Self>) -> DiffIter {
        DiffIter::from(self.clone())
    }

    pub fn extend(&self, additional: &Diff) -> Diff {
        let mut builder = DiffBuilder::default();
        builder.push_diff(self);
        builder.push_diff(additional);
        builder.finalize()
    }
        
    pub fn apply(&self, set: &Set) -> Set {
        let mut res = Vec::new();
        let mut diff = self.items.as_slice();
        let mut set = set.items.as_slice();

        // First, we process items until one of the two iterators runs out.
        while let (Some(&item), Some(&(action, payload))) = (
            set.first(), diff.first()
        ) {
            match item.cmp(&payload) {
                Ordering::Less => {
                    res.push(item);
                    skip_first(&mut set);
                }
                Ordering::Equal => {
                    if action.is_announce() {
                        res.push(item);
                    }
                    skip_first(&mut set);
                    skip_first(&mut diff);
                }
                Ordering::Greater => {
                    if action.is_announce() {
                        res.push(payload);
                    }
                    skip_first(&mut diff);
                }
            }
        }

        // Since now one of the iterators is done, only one of the two loops
        // will actually add items.
        for item in set {
            res.push(*item);
        }
        for &(action, item) in diff {
            if action.is_announce() {
                res.push(item)
            }
        }

        Set { items: res }
    }

    /*
    pub fn apply_strict(
        &self,
        set: &Set
    ) -> Result<Set, (Action, Payload)> {
        let mut res = Vec::new();
        let mut diff = self.items.as_slice();
        let mut set = set.items.as_slice();

        // First, we process items until one of the two iterators runs out.
        while let (Some(&item), Some(&(action, payload))) = (
            set.first(), diff.first()
        ) {
            match item.cmp(&payload) {
                Ordering::Less => {
                    res.push(item);
                    skip_first(&mut set);
                }
                Ordering::Equal => {
                    if action.is_announce() {
                        return Err((action, payload))
                    }
                    skip_first(&mut set);
                    skip_first(&mut diff);
                }
                Ordering::Greater => {
                    if action.is_announce() {
                        res.push(payload);
                    }
                    else {
                        return Err((action, payload))
                    }
                    skip_first(&mut diff);
                }
            }
        }

        // Since now one of the iterators is done, only one of the two loops
        // will actually add items.
        for item in set {
            res.push(*item);
        }
        for &(action, item) in diff {
            if action.is_announce() {
                res.push(item)
            }
            else {
                return Err((action, item))
            }
        }

        Ok(Set { items: res })
    }
    */
}


//------------ DiffIter -----------------------------------------------

pub struct DiffIter {
    diff: Arc<Diff>,
    pos: usize,
}

impl From<Arc<Diff>> for DiffIter {
    fn from(diff: Arc<Diff>) -> Self {
        DiffIter { diff, pos: 0 }
    }
}

impl Iterator for DiffIter {
    type Item = (Action, Payload);

    fn next(&mut self) -> Option<Self::Item> {
        match self.diff.items.get(self.pos) {
            Some(res) => {
                self.pos += 1;
                Some(*res)
            }
            None => None,
        }
    }
}


//------------ DiffBuilder --------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct DiffBuilder {
    items: Vec<(Action, Payload)>,
}

impl DiffBuilder {
    pub fn push(&mut self, action: Action, payload: Payload) {
        self.items.push((action, payload))
    }

    pub fn push_diff(&mut self, diff: &Diff) {
        self.items.extend_from_slice(&diff.items)
    }

    pub fn finalize_strict(mut self) -> Option<Diff> {
        self.items.sort_unstable_by_key(|item| item.1);
        for pair in self.items.windows(2) {
            if pair[0].1 == pair[1].1 {
                return None
            }
        }
        Some(Diff { items: self.items })
    }

    pub fn finalize(mut self) -> Diff {
        self.items.sort_unstable_by_key(|item| item.1);
        self.dedup();   
        Diff { items: self.items }
    }

    /// Removes items that are duplicates or both added and removed.
    fn dedup(&mut self) {
        // We walk over the items once via index r. Items that are to be
        // kept are swaped into their final position at index w. At the end,
        // we cut back to w items.
        let mut r = 0;
        let mut w = 0;
        
        while r < self.items.len() {
            let mut keep = true;
            let mut rr = r + 1;
            while rr < self.items.len() && self.items[rr].1 == self.items[r].1 {
                if keep {
                    if self.items[rr].0 != self.items[r].0 {
                        keep = false
                    }
                }
                rr += 1
            }
            if keep {
                self.items.swap(r, w);
                w += 1
            }
            r = rr
        }
        self.items.truncate(w);
    }
}

impl From<Diff> for DiffBuilder {
    fn from(diff: Diff) -> Self {
        DiffBuilder { items: diff.items }
    }
}


//------------ Stream --------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Stream {
    /// The name of the stream.
    name: String,

    /// The current set of payload.
    current: Option<Arc<Set>>,

    /// The diffs we remember.
    ///
    /// The newest diff is at the front.
    ///
    /// The serial number is the one from which this diff leads to current.
    /// This means that the current serial is whatever the first diff has
    /// plus 1.
    diffs: Vec<(Serial, Arc<Diff>)>,

    /// The session number of this stream.
    session: u16,

    /// The timing paramters of this stream.
    timing: Timing,

    /// The maximum number of diffs we keep.
    max_diff_count: usize,

    /// The time of last successful update.
    last_update: SystemTime,

    /// Who to tell when we have been updated.
    notify: NotifySender,
}

impl Stream {
    pub fn new(name: String, notify: NotifySender) -> Self {
        Stream {
            name,
            current: None,
            diffs: Vec::new(),
            session: {
                SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH).unwrap()
                .as_secs() as u16
            },
            timing: Default::default(),
            max_diff_count: 10,
            last_update: SystemTime::now(),
            notify,
        }
    }

    pub fn replace_set(&mut self, set: Set) {
        debug!(
            "{}: Replacing data with set of {} elements.",
            &self.name, set.len()
        );
        let diff = match self.current.as_ref() {
            Some(current) => Some(set.diff_from(current)),
            None => None,
        };
        self.update(set, diff);
    }

    pub fn add_diff(&mut self, diff: Diff) {
        debug!(
            "{}: Adding diff with {} elements.",
            &self.name, diff.len()
        );
        let current = diff.apply(self.current.as_ref().unwrap());
        self.update(current, Some(diff));
    }

    fn update(&mut self, current: Set, diff: Option<Diff>) {
        self.current = Some(Arc::new(current));
        if let Some(diff) = diff {
            let diff = Arc::new(diff);
            self.diffs.truncate(self.max_diff_count.saturating_sub(1));
            let mut diffs = Vec::with_capacity(self.diffs.len() + 1);
            diffs.push((self.serial(), diff.clone()));
            for &(serial, ref item) in &self.diffs {
                diffs.push((serial, Arc::new(item.extend(&diff))));
            }
            self.diffs = diffs;
        }
        self.last_update = SystemTime::now();
        self.notify.notify();
    }

    pub fn serial(&self) -> Serial {
        self.diffs.first().map(|item| item.0.add(1)).unwrap_or(0.into())
    }

    pub fn get_diff(&self, serial: Serial) -> Option<Arc<Diff>> {
        debug!("{}: currently at: {}", self.name, self.serial());
        debug!("{}: looking for diff from {}", self.name, serial);
        if serial == self.serial() {
            debug!("{}: Same, return empty diff.", self.name);
            Some(Arc::new(Diff::default()))
        }
        else {
            self.diffs.iter().find_map(|item| {
                if item.0 == serial {
                    debug!(
                        "{}: Trying {}. Found it!",
                        self.name, item.0
                    );
                    Some(item.1.clone())
                }
                else {
                    debug!(
                        "{}: Trying {}. That’s not it.",
                        self.name, item.0
                    );
                    None
                }
            })
        }
    }

    /*
    pub fn has_expired(&self) -> bool {
        let elapsed = self.last_update.elapsed().unwrap_or_default();
        elapsed.as_secs() > u64::from(self.timing.expire)
    }
    */
}


//------------ StreamHandle --------------------------------------------------

#[derive(Clone, Debug)]
pub struct StreamHandle(Arc<RwLock<Stream>>);

impl StreamHandle {
    pub fn new(name: String, notify: NotifySender) -> Self {
        StreamHandle(Arc::new(RwLock::new(Stream::new(name, notify))))
    }

    pub fn timing(&self) -> Timing {
        self.0.read().unwrap().timing
    }
}

impl From<Stream> for StreamHandle {
    fn from(stream: Stream) -> StreamHandle {
        StreamHandle(Arc::new(RwLock::new(stream)))
    }
}

impl VrpTarget for StreamHandle {
    type Update = StreamInput; 

    fn start(&mut self, reset: bool) -> Self::Update {
        StreamInput {
            stream: self.clone(),
            state: if reset {
                Ok(SetBuilder::default())
            }
            else {
                Err(DiffBuilder::default())
            }
        }
    }

    fn apply(
        &mut self, update: StreamInput, _reset: bool, timing: Timing
    ) -> Result<(), io::Error> {
        let mut stream = self.0.write().unwrap();
        stream.timing = timing;
        match update.state {
            Ok(set) => {
                match set.finalize_strict() {
                    Some(set) => {
                        stream.replace_set(set);
                        Ok(())
                    }
                    None => {
                        Err(io::Error::new(
                            io::ErrorKind::Other,
                            "data set with duplicates"
                        ))
                     }
                }
            }
            Err(diff) => {
                match diff.finalize_strict() {
                    Some(diff) => {
                        if !diff.is_empty() {
                            stream.add_diff(diff)
                        }
                        Ok(())
                    }
                    None => {
                        Err(io::Error::new(
                            io::ErrorKind::Other,
                            "invalid diff"
                        ))
                     }
                }
            }
        }
    }
}

impl VrpSource for StreamHandle {
    type FullIter = SetIter;
    type DiffIter = DiffIter;

    fn ready(&self) -> bool {
        self.0.read().unwrap().current.is_some()
    }

    fn notify(&self) -> State {
        let this = self.0.read().unwrap();
        State::from_parts(this.session, this.serial())
    }

    fn full(&self) -> (State, Self::FullIter) {
        let this = self.0.read().unwrap();
        match this.current.as_ref() {
            Some(current) => {
                (
                    State::from_parts(this.session, this.serial()),
                    current.clone().into()
                )
            }
            None => {
                (
                    State::from_parts(this.session, this.serial()),
                    Arc::new(Set::default()).into()
                )
            }
        }
    }

    fn diff(&self, state: State) -> Option<(State, Self::DiffIter)> {
        let this = self.0.read().unwrap();
        if this.current.is_none() || state.session() != this.session {
            return None
        }
        this.get_diff(state.serial()).map(|diff| {
            (
                State::from_parts(this.session, this.serial()),
                diff.shared_iter()
            )
        })
    }

    fn timing(&self) -> Timing {
        self.0.read().unwrap().timing
    }
}


//------------ StreamInput ---------------------------------------------------

#[derive(Clone, Debug)]
pub struct StreamInput {
    stream: StreamHandle,
    state: Result<SetBuilder, DiffBuilder>,
}

impl VrpUpdate for StreamInput { 
    fn push_vrp(&mut self, action: Action, payload: Payload) {
        match self.state {
            Ok(ref mut set) => {
                if let Action::Announce = action {
                    set.push(payload)
                }
            }
            Err(ref mut diff) => {
                    diff.push(action, payload)
            }
        }
    }
}


//------------ Helper Functions ----------------------------------------------

fn skip_first<T>(slice: &mut &[T]) {
    *slice = slice.split_first().map(|s| s.1).unwrap_or(&[])
}


//============ Testing =======================================================

#[cfg(test)]
mod test {
    use rpki_rtr::payload::Ipv4Prefix;
    use super::*;

    fn v4(addr: u32) -> Payload {
        Payload::V4(Ipv4Prefix {
            prefix: addr.into(),
            prefix_len: 32,
            max_len: 32,
            asn: 0
        })
    }

    #[test]
    fn diff_builder_dedup() {
        let mut builder = DiffBuilder::default();
        builder.push(Action::Announce, v4(2));
        builder.push(Action::Withdraw, v4(1));
        builder.push(Action::Announce, v4(2));
        builder.push(Action::Announce, v4(3));
        assert_eq!(
            builder.finalize(),
            Diff { items: vec![
                (Action::Withdraw, v4(1)),
                (Action::Announce, v4(2)),
                (Action::Announce, v4(3))
            ]}
        );

        let mut builder = DiffBuilder::default();
        builder.push(Action::Announce, v4(2));
        builder.push(Action::Withdraw, v4(1));
        builder.push(Action::Withdraw, v4(2));
        builder.push(Action::Announce, v4(2));
        builder.push(Action::Announce, v4(3));
        assert_eq!(
            builder.finalize(),
            Diff { items: vec![
                (Action::Withdraw, v4(1)),
                (Action::Announce, v4(3))
            ]}
        );

        let mut builder = DiffBuilder::default();
        builder.push(Action::Announce, v4(2));
        builder.push(Action::Withdraw, v4(1));
        builder.push(Action::Withdraw, v4(3));
        builder.push(Action::Announce, v4(2));
        builder.push(Action::Announce, v4(3));
        assert_eq!(
            builder.finalize(),
            Diff { items: vec![
                (Action::Withdraw, v4(1)),
                (Action::Announce, v4(2))
            ]}
        );
    }
}

