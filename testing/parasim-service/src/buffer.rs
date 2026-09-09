//! The reorder buffer: heads whose parent has not been accumulated yet wait here.
//!
//! Accumulate is the only authority on a para's lineage, so a head can only be applied once its
//! parent is the head that is stored. Under pipelining a package is refined against a parent that
//! is still in flight, and reports do not arrive in the order the blocks were built — so "parent
//! is not the stored head" is the normal case, not an attack. Dropping such a head would strand
//! every descendant and cost the collator a re-author of a block it already built, so accumulate
//! parks it here instead and applies it when its parent lands.
//!
//! Two rules keep the buffer from becoming a spam sink. A head is only parked if its number falls
//! in `(stored, stored + BUFFER_CAP]`, which bounds bogus futures to a plausible horizon rather
//! than merely to a count; and every invocation evicts entries the stored head has reached, which
//! is what sweeps a losing fork's children once the winning fork passes their heights.
//!
//! The ordering rules here perform no host calls: the head store is a trait the accumulate-side
//! glue implements. That is what makes them testable on the host, and it keeps the checkpoint —
//! which has to happen after *every* applied head, or a drain that runs out of gas loses its
//! progress — in the one place that knows about storage.

use alloc::vec::Vec;
use codec::{Decode, DecodeAll, Encode};
use jam_types::Slot;
use parachain_service_interface::types::HeadData;

use crate::HASH_LEN;

/// How many out-of-order heads one para may park at once.
///
/// The unincluded-segment capacity the collator builds against (3) plus one: a collator allowed
/// three unaccumulated blocks in flight can legitimately produce a fourth arrival before the first
/// one accumulates.
pub const BUFFER_CAP: usize = 4;

/// A head waiting for its parent to be accumulated.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct BufferedCandidate {
	/// blake2b-256 of the head this block was built on.
	pub parent_head_hash: [u8; HASH_LEN],
	pub head_data: HeadData,
	pub number: u32,
	/// The slot the head arrived in, which the age backstop measures against.
	pub arrived_slot: Slot,
}

impl BufferedCandidate {
	/// The hash a child of this head would name as its parent.
	pub fn head_hash(&self) -> [u8; HASH_LEN] {
		jam_state_helpers::blake2_256(&self.head_data)
	}
}

/// What the para's stored head says, as the ordering rules need it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredHead {
	/// Nothing stored: the para's first block, which has no parent to be fresh against.
	Empty,
	At {
		hash: [u8; HASH_LEN],
		number: u32,
	},
	/// Bytes are stored, but they are not a `ParaInfo` carrying a header. Reading that as an
	/// empty store is exactly the papering-over accumulate's freshness check exists to prevent,
	/// and without a number there is no window to judge an arrival against either.
	Unreadable,
}

impl StoredHead {
	/// Read what this service has stored for a para.
	pub fn read(stored: Option<&[u8]>) -> Self {
		let Some(stored) = stored else {
			return Self::Empty;
		};
		let Ok(info) = crate::ParaInfoLite::decode_all(&mut &stored[..]) else {
			return Self::Unreadable;
		};
		match crate::pov::header_number(&info.head_data) {
			Some(number) => {
				Self::At { hash: jam_state_helpers::blake2_256(&info.head_data), number }
			},
			None => Self::Unreadable,
		}
	}
}

/// Where accumulate keeps a para's head.
///
/// Implemented over the storage host calls by the service, and over a plain field by the tests.
pub trait HeadStore {
	/// The head the para is at. Read afresh rather than cached: within one accumulate call a
	/// chain of packages can land, and each one's parent is the head its predecessor just wrote.
	fn head(&self) -> StoredHead;
	/// Store `candidate`'s head, committing it against a later gas exhaustion. `false` if the
	/// write failed, which stops the drain rather than pretending the head moved.
	fn set_head(&mut self, candidate: &BufferedCandidate) -> bool;
}

/// What accumulate should do with an arriving head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
	Apply,
	Buffer,
	Drop(DropReason),
}

/// Why an arriving head is neither applied nor parked. A drop whose reason is not logged is a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
	/// The number is outside `(stored, stored + BUFFER_CAP]`: either already reached — a stale
	/// package, or a fork the winning branch has overtaken — or too far ahead to be a plausible
	/// arrival.
	OutsideNumberWindow,
	/// The same head is already parked.
	Duplicate,
	BufferFull,
	/// The stored head cannot be read, so neither lineage nor height can be judged.
	UnreadableStoredHead,
}

/// Why a parked head was thrown away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictReason {
	/// The stored head has reached the parked head's number, so its parent can never be the
	/// stored head again. This is what sweeps a losing fork's children.
	Overtaken,
	/// Parked longer than the age backstop allows; garbage collection only.
	TooOld,
}

/// Everything one arriving head did, for the caller to log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
	/// Heads written, in order: the arriving head, then the parked ones that chained onto it.
	pub applied: Vec<BufferedCandidate>,
	pub buffered: Option<BufferedCandidate>,
	pub dropped: Option<(BufferedCandidate, DropReason)>,
	pub evicted: Vec<(BufferedCandidate, EvictReason)>,
}

/// One para's parked heads, in arrival order.
#[derive(Debug, Default, Clone, PartialEq, Eq, Encode, Decode)]
pub struct ReorderBuffer(Vec<BufferedCandidate>);

impl ReorderBuffer {
	/// Decode a stored buffer. Bytes that do not decode are treated as an empty buffer: the
	/// buffer is scratch state, so losing it delays heads rather than corrupting the para's head.
	pub fn decode_or_empty(stored: Option<&[u8]>) -> Self {
		stored
			.and_then(|bytes| Self::decode_all(&mut &bytes[..]).ok())
			.unwrap_or_default()
	}

	pub fn depth(&self) -> usize {
		self.0.len()
	}

	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	/// Apply an arriving head and everything parked that chains onto it, or park it, or drop it.
	pub fn accept(
		&mut self,
		store: &mut impl HeadStore,
		arriving: BufferedCandidate,
		now: Slot,
		max_age: Slot,
	) -> Outcome {
		let mut outcome = Outcome::default();
		match self.decide(store.head(), &arriving) {
			Decision::Apply => {
				let mut next = Some(arriving);
				while let Some(candidate) = next {
					if !store.set_head(&candidate) {
						break;
					}
					next = self.take_child_of(&candidate.head_hash());
					outcome.applied.push(candidate);
				}
			},
			Decision::Buffer => {
				self.0.push(arriving.clone());
				outcome.buffered = Some(arriving);
			},
			Decision::Drop(reason) => outcome.dropped = Some((arriving, reason)),
		}
		outcome.evicted = self.evict(store.head(), now, max_age);
		outcome
	}

	/// What to do with `arriving`, given the head the para is at.
	fn decide(&self, stored: StoredHead, arriving: &BufferedCandidate) -> Decision {
		let (hash, number) = match stored {
			StoredHead::Empty => return Decision::Apply,
			StoredHead::Unreadable => return Decision::Drop(DropReason::UnreadableStoredHead),
			StoredHead::At { hash, number } => (hash, number),
		};
		if arriving.parent_head_hash == hash {
			return Decision::Apply;
		}
		if arriving.number <= number || arriving.number > number.saturating_add(BUFFER_CAP as u32) {
			return Decision::Drop(DropReason::OutsideNumberWindow);
		}
		if self.0.iter().any(|parked| parked.head_data == arriving.head_data) {
			return Decision::Drop(DropReason::Duplicate);
		}
		if self.0.len() >= BUFFER_CAP {
			return Decision::Drop(DropReason::BufferFull);
		}
		Decision::Buffer
	}

	/// Take the first-arrived parked head whose parent is `head_hash`.
	///
	/// Arrival order is what settles a tie between two siblings of the same parent: the buffer
	/// cannot tell which is the better block, so it picks the one it saw first.
	fn take_child_of(&mut self, head_hash: &[u8; HASH_LEN]) -> Option<BufferedCandidate> {
		let index = self.0.iter().position(|parked| &parked.parent_head_hash == head_hash)?;
		Some(self.0.remove(index))
	}

	fn evict(
		&mut self,
		stored: StoredHead,
		now: Slot,
		max_age: Slot,
	) -> Vec<(BufferedCandidate, EvictReason)> {
		let mut evicted = Vec::new();
		let mut kept = Vec::with_capacity(self.0.len());
		for candidate in core::mem::take(&mut self.0) {
			match eviction_reason(&candidate, stored, now, max_age) {
				Some(reason) => evicted.push((candidate, reason)),
				None => kept.push(candidate),
			}
		}
		self.0 = kept;
		evicted
	}
}

fn eviction_reason(
	candidate: &BufferedCandidate,
	stored: StoredHead,
	now: Slot,
	max_age: Slot,
) -> Option<EvictReason> {
	if let StoredHead::At { number, .. } = stored {
		if candidate.number <= number {
			return Some(EvictReason::Overtaken);
		}
	}
	(now.saturating_sub(candidate.arrived_slot) > max_age).then_some(EvictReason::TooOld)
}

#[cfg(test)]
mod tests {
	use super::*;
	use alloc::vec;

	/// A generous age backstop, so the tests that are not about it are unaffected by it.
	const MAX_AGE: Slot = 600;

	/// A head store backed by a plain field, standing in for this service's storage.
	#[derive(Default)]
	struct TestStore {
		head: Option<BufferedCandidate>,
		/// Every head written, in order — the checkpoint sequence a real drain would commit.
		writes: Vec<u32>,
		/// Makes the next `set_head` fail, as a full or over-budget storage write would.
		fail_next_write: bool,
	}

	impl HeadStore for TestStore {
		fn head(&self) -> StoredHead {
			match &self.head {
				None => StoredHead::Empty,
				Some(candidate) => {
					StoredHead::At { hash: candidate.head_hash(), number: candidate.number }
				},
			}
		}

		fn set_head(&mut self, candidate: &BufferedCandidate) -> bool {
			if core::mem::take(&mut self.fail_next_write) {
				return false;
			}
			self.writes.push(candidate.number);
			self.head = Some(candidate.clone());
			true
		}
	}

	/// A candidate whose head bytes are `head_data`, so tests can name blocks by a single byte.
	fn candidate(
		tag: u8,
		parent: [u8; HASH_LEN],
		number: u32,
		arrived_slot: Slot,
	) -> BufferedCandidate {
		BufferedCandidate {
			parent_head_hash: parent,
			head_data: vec![tag; 8].try_into().expect("8 bytes fit; qed"),
			number,
			arrived_slot,
		}
	}

	/// A chain of candidates `tag..`, each the child of the previous, starting at `number`.
	fn chain(tags: &[u8], root_parent: [u8; HASH_LEN], number: u32) -> Vec<BufferedCandidate> {
		let mut parent = root_parent;
		let mut built = Vec::new();
		for (offset, tag) in tags.iter().enumerate() {
			let next = candidate(*tag, parent, number + offset as u32, 0);
			parent = next.head_hash();
			built.push(next);
		}
		built
	}

	/// A store already at `candidate`, which is what every out-of-order test needs to start from.
	fn store_at(candidate: &BufferedCandidate) -> TestStore {
		TestStore { head: Some(candidate.clone()), ..Default::default() }
	}

	/// The base case the buffer must not disturb: heads arriving in order are applied straight
	/// away and nothing is ever parked.
	#[test]
	fn in_order_chain_works() {
		let blocks = chain(&[1, 2, 3], [0u8; HASH_LEN], 1);
		let mut store = store_at(&blocks[0]);
		let mut buffer = ReorderBuffer::default();

		for block in &blocks[1..] {
			let outcome = buffer.accept(&mut store, block.clone(), 0, MAX_AGE);
			assert_eq!(outcome.applied, vec![block.clone()]);
			assert!(buffer.is_empty());
		}
		assert_eq!(store.writes, vec![2, 3]);
	}

	/// The point of the phase: a child that arrives before its parent waits instead of being
	/// dropped, and the parent's arrival then applies both — in chain order, not arrival order.
	#[test]
	fn child_before_parent_drains_works() {
		let blocks = chain(&[1, 2, 3], [0u8; HASH_LEN], 1);
		let mut store = store_at(&blocks[0]);
		let mut buffer = ReorderBuffer::default();

		let buffered = buffer.accept(&mut store, blocks[2].clone(), 0, MAX_AGE);
		assert_eq!(buffered.buffered, Some(blocks[2].clone()));
		assert!(buffered.applied.is_empty());
		assert_eq!(buffer.depth(), 1);

		let drained = buffer.accept(&mut store, blocks[1].clone(), 0, MAX_AGE);
		assert_eq!(drained.applied, vec![blocks[1].clone(), blocks[2].clone()]);
		assert!(buffer.is_empty());
		assert_eq!(store.writes, vec![2, 3]);
	}

	/// Two siblings of the stored head: the first is applied, and the second cannot be parked —
	/// its number is one the stored head has now reached, so no future parent can accept it.
	#[test]
	fn same_parent_sibling_tie_errors() {
		let parent = candidate(1, [0u8; HASH_LEN], 1, 0);
		let winner = candidate(2, parent.head_hash(), 2, 0);
		let loser = candidate(3, parent.head_hash(), 2, 0);
		let mut store = store_at(&parent);
		let mut buffer = ReorderBuffer::default();

		assert_eq!(buffer.accept(&mut store, winner.clone(), 0, MAX_AGE).applied, vec![winner]);
		let dropped = buffer.accept(&mut store, loser.clone(), 0, MAX_AGE);
		assert_eq!(dropped.dropped, Some((loser, DropReason::OutsideNumberWindow)));
		assert!(buffer.is_empty());
	}

	/// The shape that made height eviction necessary: children of a fork that loses are parked
	/// while it is still plausible, and swept as soon as the winning fork passes their heights.
	/// Without this they would sit there until the age backstop, holding slots against the para.
	#[test]
	fn losing_fork_children_are_swept_works() {
		let root = candidate(1, [0u8; HASH_LEN], 1, 0);
		let losing = chain(&[10, 11], root.head_hash(), 2);
		let winning = chain(&[20, 21], root.head_hash(), 2);
		let mut store = store_at(&root);
		let mut buffer = ReorderBuffer::default();

		// The losing fork's grandchild arrives first, with no parent to attach to: parked.
		assert_eq!(
			buffer.accept(&mut store, losing[1].clone(), 0, MAX_AGE).buffered,
			Some(losing[1].clone())
		);
		// The winning fork then takes both heights.
		assert_eq!(
			buffer.accept(&mut store, winning[0].clone(), 0, MAX_AGE).applied,
			vec![winning[0].clone()]
		);
		assert_eq!(buffer.depth(), 1, "still plausible: number 3 is ahead of the stored head");
		let sweep = buffer.accept(&mut store, winning[1].clone(), 0, MAX_AGE);
		assert_eq!(sweep.evicted, vec![(losing[1].clone(), EvictReason::Overtaken)]);
		assert!(buffer.is_empty());
	}

	/// The cap bounds what one para can park, and the overflow is dropped with a reason rather
	/// than displacing a head that may still be about to drain.
	#[test]
	fn buffer_cap_errors() {
		let root = candidate(1, [0u8; HASH_LEN], 1, 0);
		let mut store = store_at(&root);
		let mut buffer = ReorderBuffer::default();

		// Numbers 2..=5 are the whole window, and all four have unknown parents.
		for number in 2..=1 + BUFFER_CAP as u32 {
			let orphan = candidate(number as u8 + 100, [9u8; HASH_LEN], number, 0);
			assert!(buffer.accept(&mut store, orphan, 0, MAX_AGE).buffered.is_some());
		}
		assert_eq!(buffer.depth(), BUFFER_CAP);

		let overflow = candidate(200, [9u8; HASH_LEN], 5, 0);
		let dropped = buffer.accept(&mut store, overflow.clone(), 0, MAX_AGE);
		assert_eq!(dropped.dropped, Some((overflow, DropReason::BufferFull)));
		assert_eq!(buffer.depth(), BUFFER_CAP);
	}

	/// The window, not the cap, is what makes buffer spam pointless: a head too far ahead to be a
	/// plausible arrival never occupies a slot in the first place.
	#[test]
	fn number_window_errors() {
		let root = candidate(1, [0u8; HASH_LEN], 10, 0);
		let mut store = store_at(&root);
		let mut buffer = ReorderBuffer::default();

		let too_far = candidate(2, [9u8; HASH_LEN], 10 + BUFFER_CAP as u32 + 1, 0);
		assert_eq!(
			buffer.accept(&mut store, too_far.clone(), 0, MAX_AGE).dropped,
			Some((too_far, DropReason::OutsideNumberWindow))
		);
		let already_reached = candidate(3, [9u8; HASH_LEN], 10, 0);
		assert_eq!(
			buffer.accept(&mut store, already_reached.clone(), 0, MAX_AGE).dropped,
			Some((already_reached, DropReason::OutsideNumberWindow))
		);
		assert!(buffer.is_empty());
	}

	/// The same head arriving twice — a resubmission, or the same package reported on two cores —
	/// must not take two of the four slots.
	#[test]
	fn duplicate_errors() {
		let root = candidate(1, [0u8; HASH_LEN], 1, 0);
		let orphan = candidate(2, [9u8; HASH_LEN], 3, 0);
		let mut store = store_at(&root);
		let mut buffer = ReorderBuffer::default();

		assert!(buffer.accept(&mut store, orphan.clone(), 0, MAX_AGE).buffered.is_some());
		assert_eq!(
			buffer.accept(&mut store, orphan.clone(), 0, MAX_AGE).dropped,
			Some((orphan, DropReason::Duplicate))
		);
		assert_eq!(buffer.depth(), 1);
	}

	/// A parent that never lands leaves its descendants unreachable, and height eviction cannot
	/// see them because they are ahead of the stored head. The age backstop is the only thing
	/// that frees their slots.
	#[test]
	fn age_backstop_works() {
		let root = candidate(1, [0u8; HASH_LEN], 1, 0);
		let orphan = candidate(2, [9u8; HASH_LEN], 3, 100);
		let mut store = store_at(&root);
		let mut buffer = ReorderBuffer::default();

		assert!(buffer.accept(&mut store, orphan.clone(), 100, MAX_AGE).buffered.is_some());
		// One slot short of the backstop: still parked.
		let unaffected = candidate(3, [9u8; HASH_LEN], 4, 100 + MAX_AGE);
		buffer.accept(&mut store, unaffected, 100 + MAX_AGE, MAX_AGE);
		assert_eq!(buffer.depth(), 2);

		let later = candidate(4, [9u8; HASH_LEN], 5, 100 + MAX_AGE + 1);
		let outcome = buffer.accept(&mut store, later, 100 + MAX_AGE + 1, MAX_AGE);
		assert_eq!(outcome.evicted, vec![(orphan, EvictReason::TooOld)]);
	}

	/// Nothing stored is the para's first block: it has no parent to be fresh against, so it is
	/// applied whatever it names.
	#[test]
	fn empty_store_applies_works() {
		let first = candidate(1, [7u8; HASH_LEN], 1, 0);
		let mut store = TestStore::default();
		let mut buffer = ReorderBuffer::default();

		assert_eq!(buffer.accept(&mut store, first.clone(), 0, MAX_AGE).applied, vec![first]);
	}

	/// Stored bytes that are not a readable head are not an empty store: overwriting them is the
	/// papering-over the freshness check exists to prevent.
	#[test]
	fn unreadable_stored_head_errors() {
		struct Unreadable;
		impl HeadStore for Unreadable {
			fn head(&self) -> StoredHead {
				StoredHead::Unreadable
			}
			fn set_head(&mut self, _candidate: &BufferedCandidate) -> bool {
				panic!("an unreadable head must never be overwritten")
			}
		}

		let arriving = candidate(1, [0u8; HASH_LEN], 1, 0);
		let outcome =
			ReorderBuffer::default().accept(&mut Unreadable, arriving.clone(), 0, MAX_AGE);
		assert_eq!(outcome.dropped, Some((arriving, DropReason::UnreadableStoredHead)));
	}

	/// A drain that cannot write stops there and leaves the rest parked, which is the same shape
	/// gas exhaustion produces: the heads already written stay applied (the service checkpoints
	/// after each one) and the remainder drains on a later invocation.
	#[test]
	fn interrupted_drain_resumes_works() {
		let blocks = chain(&[1, 2, 3, 4], [0u8; HASH_LEN], 1);
		let mut store = store_at(&blocks[0]);
		let mut buffer = ReorderBuffer::default();

		for block in &blocks[2..] {
			assert!(buffer.accept(&mut store, block.clone(), 0, MAX_AGE).buffered.is_some());
		}
		store.fail_next_write = true;
		let interrupted = buffer.accept(&mut store, blocks[1].clone(), 0, MAX_AGE);
		assert!(interrupted.applied.is_empty());
		assert_eq!(store.writes, Vec::<u32>::new());
		assert_eq!(buffer.depth(), 2, "the parked descendants are untouched");

		let resumed = buffer.accept(&mut store, blocks[1].clone(), 0, MAX_AGE);
		assert_eq!(resumed.applied, blocks[1..].to_vec());
		assert_eq!(store.writes, vec![2, 3, 4]);
	}

	/// The buffer survives a round trip through storage: the drain state is only useful across
	/// accumulate invocations, so an encoding change that loses it would be silent otherwise.
	#[test]
	fn storage_round_trip_works() {
		let mut buffer = ReorderBuffer::default();
		let root = candidate(1, [0u8; HASH_LEN], 1, 0);
		let orphan = candidate(2, [9u8; HASH_LEN], 3, 5);
		buffer.accept(&mut store_at(&root), orphan, 0, MAX_AGE);

		assert_eq!(ReorderBuffer::decode_or_empty(Some(&buffer.encode())), buffer);
		assert!(ReorderBuffer::decode_or_empty(Some(&[0xff, 0xff])).is_empty());
		assert!(ReorderBuffer::decode_or_empty(None).is_empty());
	}
}
