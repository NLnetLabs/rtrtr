//! The data that we are maintaining.
//!
//! We call the data we manage ‘payload,’ even if this isn’t quite accurate.
//! The data is maintained in payload sets – a collection of all the data
//! currently assumed valid – via the type [`Set`]. These sets are modified
//! by adding, removing, and updating indivual items. An atomic update to
//! a set is called a [`Diff`]. It contains all information to change a
//! given set into a new set.
//!
//! Whenever the payload set maintained by a unit changes, it sends out an
//! [`Update`]. This update contains the new payload set and a serial
//! number. This serial number is increased by one from update to update.
//!
//! The update optionally contains a payload diff with the changes from the
//! payload set of the previous update, i.e., the update with a serial number
//! one less than the current one. The diff should only be included if it is
//! available anyway or can be created cheaply. It should not be generated at
//! all cost.

use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::cmp::Ordering;
use std::sync::Arc;
use rpki_rtr::client::VrpError;
use rpki_rtr::payload::{Action, Payload};
use rpki_rtr::state::Serial;


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

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
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
                    diff.push((*source_item, Action::Withdraw));
                    skip_first(&mut source);
                }
                Ordering::Equal => {
                    skip_first(&mut source);
                    skip_first(&mut target);
                }
                Ordering::Greater => {
                    diff.push((*target_item, Action::Announce));
                    skip_first(&mut target);
                }
            }
        }

        // Now at least one set is empty so we can just withdraw anything
        // left in source and announce anything left in target. Only one of
        // those will happen.
        for &item in source {
            diff.push((item, Action::Withdraw))
        }
        for &item in target {
            diff.push((item, Action::Announce))
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

/// An iterator over the content of an arc of a set.
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

/// A builder for a payload set.
#[derive(Clone, Debug, Default)]
pub struct SetBuilder {
    items: HashSet<Payload>,
}

impl SetBuilder {
    /// Creates a new, empty set.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Inserts a new element into the set.
    ///
    /// The method fails with an appropriate error if there already is an
    /// element with the given payload in the set.
    pub fn insert(&mut self, payload: Payload) -> Result<(), VrpError> {
        if self.items.insert(payload) {
            Ok(())
        }
        else {
            Err(VrpError::DuplicateAnnounce)
        }
    }

    /// Removes an existing element from the set.
    ///
    /// The method fails with an appropriate error if there is no such item.
    pub fn remove(&mut self, payload: &Payload) -> Result<(), VrpError> {
        if self.items.remove(payload) {
            Ok(())
        }
        else {
            Err(VrpError::UnknownWithdraw)
        }
    }

    /// Returns whether the set contains the given element.
    pub fn contains(&self, payload: &Payload) -> bool {
        self.items.contains(payload)
    }

    /// Returns the number of elements currently in the set.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the set is currently empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /*
    pub fn push_set(&mut self, set: Set) {
        self.items.extend(&set.items)
    }

    pub fn finalize_strict(mut self) -> Option<Set> {
        self.items.sort_unstable();
        for pair in self.items.windows(2) {
            if pair[0] == pair[1] {
                return None
            }
        }
        Some(Set { items: self.items })
    }
    */

    /// Converts the builder into an imutable set.
    pub fn finalize(self) -> Set {
        let mut res = Set { items: self.items.into_iter().collect() };
        res.items.sort_unstable();
        res
    }
}

impl From<Set> for SetBuilder {
    fn from(set: Set) -> Self {
        SetBuilder {
            items: set.items.into_iter().collect()
        }
    }
}

impl<'a> From<&'a Set> for SetBuilder {
    fn from(set: &'a Set) -> Self {
        SetBuilder {
            items: set.items.iter().cloned().collect()
        }
    }
}


//------------ Diff ---------------------------------------------------

/// The differences between two payload sets.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Diff {
    /// The diff items.
    ///
    /// This vec is guaranteed to be ordered by payload and will only ever
    /// contain at most one element for each payload.
    items: Vec<(Payload, Action)>,
}

impl Diff {
    /// Returns the number of changes in this diff.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether this diff is empty and does not contain any changes.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Returns an iterator over a shared diff.
    pub fn shared_iter(self: &Arc<Self>) -> DiffIter {
        DiffIter::from(self.clone())
    }

    /// Returns a new diff by extending this diff with additional changes.
    ///
    /// This will result in an error if the diffs cannot be added to each
    /// other.
    pub fn extend(&self, additional: &Diff) -> Result<Diff, VrpError> {
        let mut builder = DiffBuilder::default();
        builder.push_diff(self)?;
        builder.push_diff(additional)?;
        Ok(builder.finalize())
    }

    /// Applies the diff to a set returning a new set.
    ///
    /// The method assumes that the diff can be applied cleanly to the given
    /// set.
    ///
    /// # Todo
    ///
    /// This should probably return an error if the diff cannot be applied.
    pub fn apply(&self, set: &Set) -> Set {
        let mut res = Vec::new();
        let mut diff = self.items.as_slice();
        let mut set = set.items.as_slice();

        // First, we process items until one of the two iterators runs out.
        while let (Some(&item), Some(&(payload, action))) = (
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
        for &(item, action) in diff {
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

/// An iterator over a shared diff.
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
            Some(&(payload, action)) => {
                self.pos += 1;
                Some((action, payload))
            }
            None => None,
        }
    }
}


//------------ DiffBuilder --------------------------------------------

/// A builder for a diff.
#[derive(Clone, Debug, Default)]
pub struct DiffBuilder {
    items: HashMap<Payload, Action>,
}

impl DiffBuilder {
    /// Returns the number of changes in the diff.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the diff is empty and contains no changes.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Adds a change to the diff.
    ///
    /// The method fails if there already is an action for the given payload
    /// element.
    pub fn push(
        &mut self, payload: Payload, action: Action
    ) -> Result<(), VrpError> {
        match self.items.entry(payload) {
            Entry::Vacant(entry) => {
                entry.insert(action);
                Ok(())
            }
            Entry::Occupied(entry) => {
                if *entry.get() == action {
                    Err(VrpError::Corrupt)
                }
                else {
                    entry.remove();
                    Ok(())
                }
            }
        }
    }

    /// Adds another diff to this diff.
    ///
    /// Each change in `diff` is added individually. The method will therefore
    /// fail if for any change in `diff` the payload was already present in
    /// `self`.
    pub fn push_diff(
        &mut self, diff: &Diff
    ) -> Result<(), VrpError> {
        for &(payload, action) in &diff.items {
            self.push(payload, action)?
        }
        Ok(())
    }

    /*
    pub fn finalize_strict(mut self) -> Option<Diff> {
        self.items.sort_unstable_by_key(|item| item.1);
        for pair in self.items.windows(2) {
            if pair[0].1 == pair[1].1 {
                return None
            }
        }
        Some(Diff { items: self.items })
    }
    */

    /// Converts the builder into an imutable diff.
    pub fn finalize(self) -> Diff {
        let mut res = Diff {
            items: self.items.into_iter().collect()
        };
        res.items.sort_unstable_by_key(|item| item.0);
        res
    }

    /*
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
    */
}

impl From<Diff> for DiffBuilder {
    fn from(diff: Diff) -> Self {
        DiffBuilder { items: diff.items.into_iter().collect() }
    }
}


//------------ Update --------------------------------------------------------

/// An update of a unit’s payload data.
///
/// An update keeps both the set and optional diff behind an arc and can thus
/// be copied cheaply.
#[derive(Clone, Debug)]
pub struct Update {
    /// The serial number of this update.
    serial: Serial,

    /// The new payload set.
    set: Arc<Set>,

    /// The optional diff from the previous update.
    diff: Option<Arc<Diff>>,
}

impl Update {
    /// Creates a new update.
    pub fn new(
        serial: Serial, set: Arc<Set>, diff: Option<Arc<Diff>>
    ) -> Self {
        Update { serial, set, diff }
    }

    /// Returns the serial number of the update.
    pub fn serial(&self) -> Serial {
        self.serial
    }

    /// Returns the payload set of the update.
    pub fn set(&self) -> Arc<Set> {
        self.set.clone()
    }

    /// Returns the diff if it can be used for the given serial.
    ///
    /// The method will return the diff if it is preset and if the given
    /// serial is one less than the update’s serial.
    pub fn get_usable_diff(&self, serial: Serial) -> Option<Arc<Diff>> {
        self.diff.clone().and_then(|diff| {
            if serial.add(1) == self.serial {
                Some(diff)
            }
            else {
                None
            }
        })
    }
}


//------------ Helper Functions ----------------------------------------------

fn skip_first<T>(slice: &mut &[T]) {
    *slice = slice.split_first().map(|s| s.1).unwrap_or(&[])
}

