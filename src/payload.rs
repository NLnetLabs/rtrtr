use std::slice;
use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::iter::Peekable;
use std::ops::{Deref, Range};
use std::sync::Arc;
use rpki::payload::rtr::{Action, Payload};
use rpki::rtr::client::PayloadError;
use rpki::rtr::server::{PayloadDiff, PayloadSet};
use rpki::rtr::state::Serial;


//------------ Pack ----------------------------------------------------------

/// An imutable, shareable, sorted collection of payload data.
///
/// This is essentially a vec of payload kept in an arc so it can be shared.
/// A pack always keeps the payload in sorted order. Once created, it cannot
/// be changed anymore.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pack {
    /// The payload items.
    items: Arc<[Payload]>,
}

impl Pack {
    /// Returns a slice of the payload.
    pub fn as_slice(&self) -> &[Payload] {
        self.items.as_ref()
    }

    /// Returns a block for the given range.
    ///
    /// # Panics
    ///
    /// The method panics if the range’s ends is greater than the number of
    /// items.
    pub fn block(&self, range: Range<usize>) -> Block {
        assert!(range.end <= self.items.len());
        Block {
            pack: self.clone(),
            range
        }
    }

    /// Returns an owned iterator-like for the block.
    pub fn owned_iter(&self) -> OwnedBlockIter {
        OwnedBlockIter::new(self.clone().into())
    }
}


//--- Default

impl Default for Pack {
    fn default() -> Self {
        Pack { items: Arc::new([]) }
    }
}


//--- Deref, AsRef, Borrow

impl Deref for Pack {
    type Target = [Payload];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef<[Payload]> for Pack {
    fn as_ref(&self) -> &[Payload] {
        self.as_slice()
    }
}

impl Borrow<[Payload]> for Pack {
    fn borrow(&self) -> &[Payload] {
        self.as_slice()
    }
}


//------------ PackBuilder ---------------------------------------------------

/// A builder for a payload pack.
#[derive(Clone, Debug, Default)]
pub struct PackBuilder {
    items: HashSet<Payload>,
}

impl PackBuilder {
    /// Creates a new, empty set.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Inserts a new element into the set.
    ///
    /// The method fails with an appropriate error if there already is an
    /// element with the given payload in the set.
    pub fn insert(&mut self, payload: Payload) -> Result<(), PayloadError> {
        if self.items.insert(payload) {
            Ok(())
        }
        else {
            Err(PayloadError::DuplicateAnnounce)
        }
    }

    /// Inserts a new element without checking.
    pub fn insert_unchecked(&mut self, payload: Payload) {
        self.items.insert(payload);
    }

    /// Removes an existing element from the set.
    ///
    /// The method fails with an appropriate error if there is no such item.
    pub fn remove(&mut self, payload: &Payload) -> Result<(), PayloadError> {
        if self.items.remove(payload) {
            Ok(())
        }
        else {
            Err(PayloadError::UnknownWithdraw)
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

    /// Converts the builder into an imutable set.
    pub fn finalize(self) -> Pack {
        let mut items: Vec<_> = self.items.into_iter().collect();
        items.sort_unstable();
        Pack { items: items.into_boxed_slice().into() }
    }
}


//------------ Block ---------------------------------------------------------

/// Part of a [`Pack`].
///
/// A block references a slice of a [`Pack`]’s items.
#[derive(Clone, Debug)]
pub struct Block {
    pack: Pack,
    range: Range<usize>,
}

impl Block {
    /// Returns the first index in the underlying pack.
    pub fn start(&self) -> usize {
        self.range.start
    }

    /// Returns the first index in the pack that is not in the block.
    pub fn end(&self) -> usize {
        self.range.end
    }

    /// Returns the block’s content as a slice,
    fn as_slice(&self) ->&[Payload] {
        &self.pack[self.range.clone()]
    }

    /// Returns an item from the pack if the index is in range.
    pub(crate) fn get_from_pack(&self, pack_index: usize) -> Option<&Payload> {
        if self.range.contains(&pack_index) {
            self.pack.get(pack_index)
        }
        else {
            None
        }
    }

    /// Returns a block from the beginning to the given pack index.
    ///
    /// # Panics
    ///
    /// The method panics if `pack_index` is beyond the end of the block.
    pub(crate) fn head_until(&self, pack_index: usize) -> Self {
        assert!(pack_index <= self.range.end);
        Block {
            pack: self.pack.clone(),
            range: self.range.start..pack_index
        }
    }

    /// Returns an owned iterator-like for the block.
    pub fn owned_iter(&self) -> OwnedBlockIter {
        OwnedBlockIter::new(self.clone())
    }

    /// Returns whether this block overlaps in content with the given block.
    pub fn overlaps(&self, other: &Block) -> bool {
        // Since blocks are not continous, we really have to check item
        // pairs. But because they are ordered, we can optimize this a bit.
        // We’ll go over self item for item and advance other until the first
        // item that is bigger -- or equal, in which case we have an overlap.
        let mut other_iter = other.iter().peekable();
        for self_item in self.iter() {
            loop {
                let other_item = match other_iter.peek() {
                    Some(item) => item,
                    None => return false
                };
                match other_item.cmp(&self_item) {
                    Ordering::Less => {
                        let _ = other_iter.next();
                    }
                    Ordering::Equal => return true,
                    Ordering::Greater => break,
                }
            }
        }
        false
    }
}


//--- From

impl From<Pack> for Block {
    fn from(pack: Pack) -> Self {
        Block {
            range: 0..pack.len(),
            pack,
        }
    }
}


//--- Deref, AsRef, and Borrow

impl Deref for Block {
    type Target = [Payload];

    fn deref(&self) -> &[Payload] {
        self.as_slice()
    }
}

impl AsRef<[Payload]> for Block {
    fn as_ref(&self) ->&[Payload] {
        self.as_slice()
    }
}

impl Borrow<[Payload]> for Block {
    fn borrow(&self) ->&[Payload] {
        self.as_slice()
    }
}


//--- IntoIterator

impl<'a> IntoIterator for &'a Block {
    type Item = &'a Payload;
    type IntoIter = slice::Iter<'a, Payload>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}


//------------ OwnedBlockIter ------------------------------------------------

/// An owned iterator-like type for iterating over the items of a block.
#[derive(Clone, Debug)]
pub struct OwnedBlockIter {
    block: Block,
    pos: usize,
}

impl OwnedBlockIter {
    /// Creates a new value.
    fn new(block: Block) -> Self {
        OwnedBlockIter {
            pos: block.range.start,
            block
        }
    }

    /// Peeks at the next item.
    pub fn peek(&self) -> Option<&Payload> {
        if self.pos < self.block.range.end {
            self.block.pack.get(self.pos)
        }
        else {
            None
        }
    }

    /// Returns the next item.
    ///
    /// This is similar to an iterator but returns a reference to the item
    /// instead of a clone.
    #[allow(clippy::should_implement_trait)] // The name is on purpose.
    pub fn next(&mut self) -> Option<&Payload> {
        if self.pos < self.block.range.end {
            let res = self.block.pack.get(self.pos)?;
            self.pos +=1;
            Some(res)
        }
        else {
            None
        }
    }
}


//------------ Set -----------------------------------------------------------

/// An ordered set of payload items.
#[derive(Clone, Debug)]
pub struct Set {
    /// The blocks of the set.
    blocks: Arc<[Block]>,

    /// The overall number of items in the set.
    len: usize,
}

impl Set {
    /// Returns the number of items in the set.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Returns an iterator over the set’s elements.
    pub fn iter(&self) -> SetIter {
        SetIter::new(self)
    }

    /// Returns an owned iterator over the set’s elements.
    pub fn owned_iter(&self) -> OwnedSetIter {
        OwnedSetIter::new(self.clone())
    }

    /// Converts the set into an owned iterator.
    pub fn into_owned_iter(self) -> OwnedSetIter {
        OwnedSetIter::new(self)
    }

    /// Returns a set which has this set and the other set merged.
    ///
    /// The two sets may overlap.
    pub fn merge(&self, other: &Set) -> Set {
        let mut res = self.to_builder();
        res.insert_set(other.clone());
        res.finalize()
    }

    /// Returns a set with the indicated elements removed.
    ///
    /// Each element in the current set is presented to the closure and only
    /// those for which the closure returns `true` are added to the returned
    /// set.
    pub fn filter(&self, mut retain: impl FnMut(&Payload) -> bool) -> Set {
        let mut res = Vec::new();
        let mut res_len = 0;

        // Here’s the idea: We go over the blocks and for each blocks we
        // cycle between looking for the first element to drop and then for
        // the first element to retain. We add the runs that are to be
        // retained to `res` and ignore the ones to be dropped.
        for block in self.blocks.iter() {
            let mut start = block.start();
            while start < block.end() {
                // A block to retain ...
                let mut end = start;
                while end < block.end() {
                    if !retain(&block.pack[end]) {
                        break;
                    }
                    else {
                        end += 1;
                    }
                }
                if end > start {
                    let new_block = block.pack.block(start..end);
                    res_len += new_block.len();
                    res.push(new_block);
                }

                // ... a block to ignore.
                end += 1;
                while end < block.end() {
                    if retain(&block.pack[end]) {
                        break;
                    }
                    else {
                        end += 1;
                    }
                }
                start = end;
            }
        }

        Set {
            blocks: res.into(),
            len: res_len
        }
    }

    /// Returns the diff to get from `other` to `self`.
    pub fn diff_from(&self, other: &Set) -> Diff {
        let mut diff = DiffBuilder::empty();
        let mut source = other.iter().peekable();
        let mut target = self.iter().peekable();

        // Process items while there’s some left in both sets.
        while let (Some(&source_item), Some(&target_item)) = (
            source.peek(), target.peek()
        ) {
            match source_item.cmp(target_item) {
                Ordering::Less => {
                    diff.withdrawn.insert_unchecked(source_item.clone());
                    source.next();
                }
                Ordering::Equal => {
                    source.next();
                    target.next();
                }
                Ordering::Greater => {
                    diff.announced.insert_unchecked(target_item.clone());
                    target.next();
                }
            }
        }

        // Now at least one set is empty so we can just withdraw anything
        // left in source and announce anything left in target. Only one of
        // those will happen.
        for item in source {
            diff.withdrawn.insert_unchecked(item.clone());
        }
        for item in target {
            diff.announced.insert_unchecked(item.clone());
        }
        diff.finalize()
    }


    /// Returns a reference of the blocks of the set.
    pub fn as_blocks(&self) -> &[Block] {
        self.blocks.as_ref()
    }

    /// Returns a builder based on the set.
    pub fn to_builder(&self) -> SetBuilder {
        SetBuilder {
            blocks: self.blocks.as_ref().into()
        }
    }
}


//--- Default

impl Default for Set {
    fn default() -> Self {
        Set {
            blocks: Arc::new([]),
            len: 0,
        }
    }
}


//--- From

impl From<Pack> for Set {
    fn from(pack: Pack) -> Self {
        Block::from(pack).into()
    }
}

impl From<Block> for Set {
    fn from(block: Block) -> Self {
        Set {
            len: block.len(),
            blocks: vec!(block).into(),
        }
    }
}


//--- PartialEq and Eq

impl PartialEq for Set {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl Eq for Set { }


//--- IntoIterator

impl<'a> IntoIterator for &'a Set {
    type Item = &'a Payload;
    type IntoIter = SetIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        SetIter::new(self)
    }
}


//------------ SetIter -------------------------------------------------------

/// An iterator over the content of a set.
#[derive(Clone, Debug)]
pub struct SetIter<'a> {
    /// The “block” we currently are processing.
    head: &'a [Payload],
 
    /// During iteration, we modify the block’s ranges
    /// The blocks we haven’t processed yet.
    tail: &'a [Block],
}

impl<'a> SetIter<'a> {
    fn new(set: &'a Set) -> Self {
        let mut res = SetIter {
            head: &[],
            tail: &set.blocks
        };
        res.next_block();
        res
    }

    /// Progresses to the next block.
    ///
    /// Returns `true` if there is another block to progress to.
    fn next_block(&mut self) -> bool {
        match self.tail.split_first() {
            Some((head, tail)) => {
                self.head = head.as_slice();
                self.tail = tail;
                true
            }
            None => false,
        }
    }
}

impl<'a> Iterator for SetIter<'a> {
    type Item = &'a Payload;

    fn next(&mut self) -> Option<Self::Item> {
        match self.head.split_first() {
            Some((head, tail)) => {
                self.head = tail;
                Some(head)
            }
            None => {
                if self.next_block() {
                    self.next()
                }
                else {
                    None
                }
            }
        }
    }
}


//------------ OwnedSetIter --------------------------------------------------

/// An owned iterator-like over the content of an arc of a set.
#[derive(Clone, Debug)]
pub struct OwnedSetIter {
    set: Set,
    block: usize,
    item: usize,
}

impl OwnedSetIter {
    fn new(set: Set) -> Self {
        OwnedSetIter {
            set, block: 0, item: 0
        }
    }

    /// Peeks at the next item.
    pub fn peek(&self) -> Option<&Payload> {
        if let Some(item) =
            self.set.blocks.get(self.block)?.get_from_pack(self.item)
        {
            Some(item)
        }
        else {
            self.set.blocks.get(self.block + 1)?.first()
        }
    }
}

impl PayloadSet for OwnedSetIter {
    fn next(&mut self) -> Option<&Payload> {
        if let Some(item) =
            self.set.blocks.get(self.block)?.get_from_pack(self.item)
        {
            self.item += 1;
            Some(item)
        }
        else {
            self.block += 1;
            self.item = self.set.blocks.get(self.block)?.start();
            self.set.blocks.get(self.block)?.get_from_pack(self.item)
        }
    }
}


//------------ SetBuilder-----------------------------------------------------

/// A builder for a set.
#[derive(Clone, Debug, Default)]
pub struct SetBuilder {
    blocks: Vec<Block>,
}

impl SetBuilder {
    /// Creates a new empty builder.
    pub fn empty() -> Self {
        Default::default()
    }

    /// Inserts a pack into the builder.
    pub fn insert_pack(&mut self, pack: Pack) {
        self.insert_block(pack.into());
    }

    /// Inserts a block into the builder.
    pub fn insert_block(&mut self, block: Block) {
        self.blocks.push(block);
    }

    /// Inserts a set into the builder.
    pub fn insert_set(&mut self, set: Set) {
        self.blocks.extend(set.blocks.iter().cloned())
    }

    /// Inserts a pack into the builder if it doesn’t overlap.
    pub fn try_insert_pack(&mut self, pack: Pack) -> Result<(), PayloadError> {
        self.try_insert_block(pack.into())
    }

    /// Inserts a block into the builder if it doesn’t overlap.
    pub fn try_insert_block(
        &mut self, block: Block
    ) -> Result<(), PayloadError> {
        if self.blocks.iter().any(|item| item.overlaps(&block)) {
            return Err(PayloadError::Corrupt)
        }
        self.insert_block(block);
        Ok(())
    }

    /// Finalizes the builder into a set.
    pub fn finalize(mut self) -> Set {
        // All blocks themselves are already sorted. But since they may not
        // be continuous, we may have to break them up and insert other
        // blocks in between.

        // Now we take a slice of the blocks vec and eat blocks from the
        // beginning. We trim the unique sections off the first block and
        // add them to the result. Rinse and repeat until the slice is empty.
        let mut res = Vec::new();
        let mut res_len = 0;
        let mut src = self.blocks.as_mut_slice();
        loop {
            // First, let’s skip over all empty blocks at the beginning. We
            // can use this later and slowly drain the first block until it
            // is empty and gets removed here.
            while src.first().map(|blk| blk.is_empty()).unwrap_or(false) {
                src = &mut src[1..];
            }

            // Next, sorts the blocks by their start element. This is
            // necessary since later we will cut elements from the start of
            // the first block and that may result in it having to go further
            // back.
            src.sort_by(|left, right| left.first().cmp(&right.first()));

            // Because of lifetimes we can only manipulate `src` once we
            // dropped all references into it. So we just calculate the end of 
            // the part of the first block we have pushed to the result
            // already and then deal with it later.
            let first_end = {
                // First element or we are done.
                let first = match src.first() {
                    Some(first) => first,
                    None => break,
                };

                // Get the first element of the next non-empty block. If there
                // isn’t one, we can append the whole first block and be done.
                let second = match
                    src[1..].iter().find_map(|blk| blk.first())
                {
                    Some(second) => second,
                    None => {
                        res.push(first.clone());
                        res_len += first.len();
                        break;
                    }
                };

                // Find the last element in `first` that is smaller
                // than `second`. Note that we are working with pack indexes
                // here so we can more easily split blocks later.
                let mut first_end = first.start();
                while let Some(item) = first.get_from_pack(first_end) {
                    if item < second {
                        first_end += 1;
                    }
                    else {
                        break;
                    }
                }
                
                // Add the part before `first_end` to the result.
                if first_end > first.start() {
                    let block = first.head_until(first_end);
                    res_len += block.len();
                    res.push(block);
                }

                // If the first remaining element of `first` is equal to
                // the first element in `second`, we need to skip it, too.
                if first.get_from_pack(first_end) == Some(second) {
                    first_end + 1
                }
                else {
                    first_end
                }
            };

            // The beginning of the first block needs to be `first_end` now.
            if let Some(first) = src.first_mut() {
                first.range.start = first_end;
            }
        }

        Set {
            blocks: res.into(),
            len: res_len
        }
    }
}


//------------ Diff ----------------------------------------------------------

/// The differences between two payload sets.
///
/// This is a list of additions to a set called _announcments_ and a list of
/// removals called _withdrawals._ When iterated over, these two are provided
/// as a single list of pairs of [`Payload`] and [`Action`]s in order of the
/// payload. This makes it relatively safe to apply non-atomically.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Diff {
    /// A set of announced elements.
    announced: Pack,

    /// A pack of withdrawn elements.
    withdrawn: Pack,
}

impl Diff {
    /// Returns the number of changes in the diff.
    pub fn len(&self) -> usize {
        self.announced.len() + self.withdrawn.len()
    }

    /// Returns whether the diff contains no changes at all.
    pub fn is_empty(&self) -> bool {
        self.announced.is_empty() && self.withdrawn.is_empty()
    }

    /// Returns an iterator over the set’s elements.
    pub fn iter(&self) -> DiffIter {
        DiffIter::new(self)
    }

    /// Returns an owned iterator over the diff’s elements.
    pub fn owned_iter(&self) -> OwnedDiffIter {
        OwnedDiffIter::new(self.clone())
    }

    /// Converts the value into an owned iterator.
    pub fn into_owned_iter(self) -> OwnedDiffIter {
        OwnedDiffIter::new(self)
    }

    /// Returns a new diff by extending this diff with additional changes.
    ///
    /// This will result in an error if the diffs cannot be added to each
    /// other.
    pub fn extend(&self, additional: &Diff) -> Result<Diff, PayloadError> {
        let mut builder = DiffBuilder::default();
        builder.push_diff(self)?;
        builder.push_diff(additional)?;
        Ok(builder.finalize())
    }

    /// Applies the diff to a set returning a new set.
    #[allow(clippy::mutable_key_type)] // false positive on Payload.
    pub fn apply(&self, set: &Set) -> Result<Set, PayloadError> {
        let mut res = set.to_builder();
        res.try_insert_pack(self.announced.clone()).map_err(|_|
            PayloadError::DuplicateAnnounce
        )?;
        let res = res.finalize();
        let mut withdrawn: HashSet<_> = self.withdrawn.iter().collect();
        let res = res.filter(|item| {
            !withdrawn.remove(item)
        });
        if !withdrawn.is_empty() {
            Err(PayloadError::UnknownWithdraw)
        }
        else {
            Ok(res)
        }
    }

    /// Applies the diff to a set ignoring overlaps and missing items.
    #[allow(clippy::mutable_key_type)] // false positive on Payload.
    pub fn apply_relaxed(&self, set: &Set) -> Set {
        let mut res = set.to_builder();
        res.insert_pack(self.announced.clone());
        let res = res.finalize();
        let mut withdrawn: HashSet<_> = self.withdrawn.iter().collect();
        res.filter(|item| {
            !withdrawn.remove(item)
        })
    }
}

impl<'a> IntoIterator for &'a Diff {
    type Item = (&'a Payload, Action);
    type IntoIter = DiffIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}


//------------ DiffIter ------------------------------------------------------

/// An iterator over the content of a diff.
#[derive(Clone, Debug)]
pub struct DiffIter<'a> {
    announced: Peekable<slice::Iter<'a, Payload>>,
    withdrawn: Peekable<slice::Iter<'a, Payload>>,
}

impl<'a> DiffIter<'a> {
    fn new(diff: &'a Diff) -> Self {
        DiffIter {
            announced: diff.announced.iter().peekable(),
            withdrawn: diff.withdrawn.iter().peekable(),
        }
    }
}

impl<'a> Iterator for DiffIter<'a> {
    type Item = (&'a Payload, Action);

    fn next(&mut self) -> Option<Self::Item> {
        match (self.announced.peek(), self.withdrawn.peek()) {
            (Some(_), None) => {
                self.announced.next().map(|some| (some, Action::Announce))
            }
            (None, Some(_)) => {
                self.withdrawn.next().map(|some| (some, Action::Withdraw))
            }
            (Some(announced), Some(withdrawn)) => {
                if announced < withdrawn {
                    self.announced.next().map(|some| (some, Action::Announce))
                }
                else {
                    self.withdrawn.next().map(|some| (some, Action::Withdraw))
                }
            }
            (None, None) => None,
        }
    }
}


//------------ OwnedDiffIter -------------------------------------------------

/// An owned iterator-like over the content of diff.
#[derive(Clone, Debug)]
pub struct OwnedDiffIter {
    announced: OwnedBlockIter,
    withdrawn: OwnedBlockIter,
}

impl OwnedDiffIter {
    fn new(diff: Diff) -> Self {
        OwnedDiffIter {
            announced: diff.announced.owned_iter(), 
            withdrawn: diff.withdrawn.owned_iter(),
        }
    }

    /// Peeks at the next item.
    pub fn peek(&self) -> Option<(&Payload, Action)> {
        match (self.announced.peek(), self.withdrawn.peek()
        ) {
            (Some(some), None) => Some((some, Action::Announce)),
            (None, Some(some)) => Some((some, Action::Withdraw)),
            (Some(announced), Some(withdrawn)) => {
                if announced < withdrawn {
                    Some((announced, Action::Announce))
                }
                else {
                    Some((withdrawn, Action::Withdraw))
                }
            }
            (None, None) => None,
        }
    }
}

impl PayloadDiff for OwnedDiffIter {
    fn next(&mut self) -> Option<(&Payload, Action)> {
        match (self.announced.peek(), self.withdrawn.peek()
        ) {
            (Some(_), None) => {
                self.announced.next().map(|some| (some, Action::Announce))
            }
            (None, Some(_)) => {
                self.withdrawn.next().map(|some| (some, Action::Withdraw))
            }
            (Some(announced), Some(withdrawn)) => {
                if announced < withdrawn {
                    self.announced.next().map(|some| (some, Action::Announce))
                }
                else {
                    self.withdrawn.next().map(|some| (some, Action::Withdraw))
                }
            }
            (None, None) => None,
        }
    }
}


//------------ DiffBuilder ---------------------------------------------------

/// A builder for a diff.
#[derive(Clone, Debug, Default)]
pub struct DiffBuilder {
    announced: PackBuilder,
    withdrawn: PackBuilder,
}

impl DiffBuilder {
    /// Creates an empty builder.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the number of changes in the diff.
    pub fn len(&self) -> usize {
        self.announced.len() + self.withdrawn.len()
    }

    /// Returns whether the builder is currently empty.
    pub fn is_empty(&self) -> bool {
        self.announced.is_empty() && self.withdrawn.is_empty()
    }

    /// Adds a change to the diff.
    ///
    /// The method fails if there already is an action for the given payload
    /// element.
    pub fn push(
        &mut self, payload: Payload, action: Action
    ) -> Result<(), PayloadError> {
        match action {
            Action::Announce => {
                if self.withdrawn.contains(&payload) {
                    return Err(PayloadError::Corrupt)
                }
                self.announced.insert(payload)
            }
            Action::Withdraw => {
                if self.announced.contains(&payload) {
                    return Err(PayloadError::Corrupt)
                }
                self.withdrawn.insert(payload)
            }
        }
    }

    /// Adds another diff to this diff.
    ///
    /// The `diff` is added as if it were the next step in a chain of diffs.
    /// That is, if it announces elements previously withdrawn or withdraws
    /// elements previously announced, these are simply dropped from the
    /// builder.
    pub fn push_diff(
        &mut self, diff: &Diff
    ) -> Result<(), PayloadError> {
        for (payload, action) in diff {
            match action {
                Action::Announce => {
                    if self.withdrawn.remove(payload).is_err() {
                        self.announced.insert(payload.clone())?
                    }
                }
                Action::Withdraw => {
                    if self.announced.remove(payload).is_err() {
                        self.withdrawn.insert(payload.clone())?
                    }
                }
            }
        }
        Ok(())
    }

    /// Converts the builder into a diff.
    pub fn finalize(self) -> Diff {
        Diff {
            announced: self.announced.finalize(),
            withdrawn: self.withdrawn.finalize(),
        }
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
    set: Set,

    /// The optional diff from the previous update.
    diff: Option<Diff>,
}

impl Update {
    /// Creates a new update.
    pub fn new(
        serial: Serial, set: Set, diff: Option<Diff>
    ) -> Self {
        Update { serial, set, diff }
    }

    /// Returns the serial number of the update.
    pub fn serial(&self) -> Serial {
        self.serial
    }

    /// Returns the payload set of the update.
    pub fn set(&self) -> &Set {
        &self.set
    }

    /// Converts the update into the payload set.
    pub fn into_set(self) -> Set {
        self.set
    }

    /// Returns the diff if it can be used for the given serial.
    ///
    /// The method will return the diff if it is preset and if the given
    /// serial is one less than the update’s serial.
    pub fn get_usable_diff(&self, serial: Serial) -> Option<&Diff> {
        self.diff.as_ref().and_then(|diff| {
            if serial.add(1) == self.serial {
                Some(diff)
            }
            else {
                None
            }
        })
    }

    /// Applies a diff to the update.
    ///
    /// The update will retain its current serial number.
    pub fn apply_diff_relaxed(&mut self, diff: &Diff)  {
        self.set = diff.apply_relaxed(&self.set);
        self.diff = None;
    }
}


//============ Tests =========================================================

#[cfg(test)]
pub(crate) mod testrig {
    use super::*;
    use std::net::IpAddr;

    
    //-------- Scaffolding ---------------------------------------------------

    /// Create payload from a `u32`.
    ///
    /// We assume that the ordering of payload is correct, so it is fine to
    /// only use the most simple type of payload, an IPv4 VRP. To further
    /// simplify things, this function makes such a VRP from a `u32` in some
    /// arbitrary way.
    pub fn p(value: u32) -> Payload {
        Payload::origin(
            rpki::payload::addr::MaxLenPrefix::new(
                rpki::payload::addr::Prefix::new_v4(value.into(), 32).unwrap(),
                Some(32)
            ).unwrap(),
            0.into()
        )
    }

    /// Create a pack of payload from a slice of `u32`s.
    pub fn pack(values: &[u32]) -> Pack {
        Pack {
            items:
                values.iter().cloned().map(p).collect::<Vec<_>>().into()
        }
    }

    /// Create a block of payload from a slice of `u32`s.
    pub fn block(values: &[u32], range: Range<usize>) -> Block {
        Block {
            pack: pack(values),
            range
        }
    }

    /// Checks that a pack fulfils all invariants.
    pub fn check_pack(pack: &Pack) {
        // Empty pack is allowed.
        if pack.items.is_empty() {
            return
        }

        // Pack needs to be ordered without duplicates.
        for window in pack.items.windows(2) {
            assert!(window[0] < window[1])
        }
    }

    /// Checks that a set conforms to all invariants.
    pub fn check_set(set: &Set) {
        // No empty blocks.
        for block in set.blocks.iter() {
            assert!(!block.is_empty())
        }

        // Elements are in order and without duplicates.
        //
        // (This relies on SetIter being correct -- there are tests for that
        // below.)
        for window in set.iter().cloned().collect::<Vec<_>>().windows(2) {
            assert!(window[0] < window[1])
        }
    }

    /// Converts a set into a vec of integers.
    pub fn set_to_vec(set: &Set) -> Vec<u32> {
        set.iter().map(|payload| match payload {
            Payload::Origin(item) => {
                match item.prefix.addr() {
                    IpAddr::V4(addr) => addr.into(),
                    _ => panic!("not a v4 prefix")
                }
            }
            _ => panic!("not a v4 prefix")
        }).collect()
    }
}


#[cfg(test)]
mod test {
    use super::*;
    use super::testrig::*;

    #[test]
    fn set_iter() {
        assert_eq!(
            Set {
                blocks: vec![
                    block(&[1, 2, 4], 0..3),
                    block(&[4, 5], 1..2)
                ].into(),
                len: 4
            }.iter().cloned().collect::<Vec<_>>(),
            [p(1), p(2), p(4), p(5)]
        );
    }

    #[test]
    fn set_builder() {
        let mut builder = SetBuilder::empty();
        builder.insert_pack(pack(&[1, 2, 11, 12]));
        builder.insert_pack(pack(&[5, 6, 7, 15, 18]));
        builder.insert_pack(pack(&[6, 7]));
        builder.insert_pack(pack(&[7]));
        builder.insert_pack(pack(&[17]));
        let set = builder.finalize();
        check_set(&set);
        assert_eq!(
            set_to_vec(&set),
            [1, 2, 5, 6, 7, 11, 12, 15, 17, 18]
        );
    }

    #[test]
    fn diff_iter() {
        use rpki::payload::rtr::Action::{Announce as A, Withdraw as W};

        assert_eq!(
            Diff {
                announced: pack(&[6, 7, 15, 18]).into(),
                withdrawn: pack(&[2, 8, 9]).into(),
            }.iter().collect::<Vec<_>>(),
            [
                (&p(2), W), (&p(6), A), (&p(7), A), (&p(8), W), (&p(9), W),
                (&p(15), A), (&p(18), A)
            ]
        );
    }

    #[test]
    fn mix_and_match() {
        use rand::Rng;
        
        fn random_vec<T: Rng>(rng: &mut T, len: usize) -> Vec<Payload> {
            let mut res = Vec::with_capacity(len);
            for _ in 0..len {
                res.push(p(rng.gen()))
            }
            res
        }

        fn build_pack(data: &[Payload]) -> Pack {
            let mut res = PackBuilder::empty();
            for item in data {
                res.insert_unchecked(item.clone());
            }
            let res = res.finalize();
            check_pack(&res);
            res
        }

        fn sort_and_dedup(mut vec: Vec<Payload>) -> Vec<Payload> {
            vec.sort();
            vec.dedup();
            vec
        }

        // Let’s make three vecs with payload data.
        let mut rng = rand_pcg::Pcg32::new(
            0xcafef00dd15ea5e5, 0xa02bdbf7bb3c0a7
        );
        let v1 = random_vec(&mut rng, 100);
        let v2 = random_vec(&mut rng, 10);
        let v3 = random_vec(&mut rng, 50);

        // Make packs from the vecs, check that they are the same as the vecs.
        let p1 = build_pack(&v1);
        let p2 = build_pack(&v2);
        let p3 = build_pack(&v3);
        let v1 = sort_and_dedup(v1);
        let v2 = sort_and_dedup(v2);
        let v3 = sort_and_dedup(v3);
        assert!(p1.iter().eq(v1.iter()));
        assert!(p2.iter().eq(v2.iter()));
        assert!(p3.iter().eq(v3.iter()));

        // Now merge everything into one vec and one set and see if
        // they match.
        let mut v = v1.clone();
        v.extend_from_slice(&v2);
        v.extend_from_slice(&v3);
        v.sort();
        v.dedup();
        let v = v;

        let mut s = SetBuilder::empty();
        s.insert_pack(p1.clone());
        s.insert_pack(p2.clone());
        s.insert_pack(p3.clone());
        let s = s.finalize();

        assert!(s.iter().eq(v.iter()));

        // Now make diffs and see if they are correct.
        let h1 = v1.iter().cloned().collect::<HashSet<_>>();
        let h2 = v2.iter().cloned().collect::<HashSet<_>>();
        let h3 = v3.iter().cloned().collect::<HashSet<_>>();
        let s1 = Set::from(p1.clone());
        let s2 = Set::from(p2.clone());
        let s3 = Set::from(p3.clone());
        let d2 = s2.diff_from(&s1);
        let d3 = s3.diff_from(&s2);

        fn check_diff(
            d: &Diff, from: &HashSet<Payload>, to: &HashSet<Payload>
        ) {
            let mut announced =
                to.difference(from).cloned().collect::<Vec<_>>();
            announced.sort();
            let mut withdrawn =
                from.difference(to).cloned().collect::<Vec<_>>();
            withdrawn.sort();
            assert!(d.announced.iter().eq(announced.iter()));
            assert!(d.withdrawn.iter().eq(withdrawn.iter()));
        }
        check_diff(&d2, &h1, &h2);
        check_diff(&d3, &h2, &h3);

        // Now check that applying those diffs works.
        assert!(d2.apply(&s1).unwrap().iter().eq(s2.iter()));
        assert!(d3.apply(&s2).unwrap().iter().eq(s3.iter()));

        // Now merge the two diffs and see if that still works.
        assert!(
            d2.extend(&d3).unwrap().apply(&s1).unwrap().iter().eq(s3.iter())
        );
    }
}

