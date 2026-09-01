use num_enum::{IntoPrimitive, TryFromPrimitive};

/// Host call indices as per §4.3.
///
/// Every host function is imported at a fixed index. Those forwarding a JAM host
/// call keep its Gray Paper index; those native to the Parachain Service are
/// numbered from 100 up.
///
/// NOTE: these are the indices the **child PVF** imports at, which this service
/// decodes — a different index space from the ones the service itself uses to
/// call into JAM. The vendored PolkaJAM host numbers its own calls `gas` 0,
/// `fetch` 1, `historical_lookup` 6, `export` 7, because it has no `grow_heap`
/// at 1. The two sets look alike; conflating them is a silent ABI break.
#[derive(IntoPrimitive, TryFromPrimitive)]
#[repr(u64)]
pub enum HostCall {
	// --- JAM host functions, forwarded unchanged (§4.3) ---
	/// The remaining gas budget.
	Gas = 0,
	/// Expand the RW data region.
	GrowHeap = 1,
	/// Read the work package and its context: the package itself, the refine
	/// context, the authorizer config and token, the work-item summaries and
	/// payloads, and the import segments.
	Fetch = 2,
	/// Read a service's preimage store at the lookup-anchor; serves both own and
	/// foreign lookups.
	HistoricalLookup = 7,
	/// Write a segment to the JAM Data Lake, e.g. an outbound XCMP payload.
	Export = 8,

	// --- Parachain Service host functions (§4.3) ---
	/// Declare the parent head hash this candidate was built on.
	SetParentHeadHash = 100,
	/// Declare the new head data this parachain block produced.
	SetHead = 101,
	/// Append one upward message to the work digest.
	SendUpwardMessage = 102,
	/// Abort the PVF with an opaque error payload.
	ReportError = 103,
}

#[cfg(test)]
mod tests {
	use super::*;

	// The discriminants are the child's import indices (§4.3). The mirror in
	// `runtimes/frameless/src/lib.rs` `host` is `#[cfg(target_arch = "riscv64")]`
	// and never compiled here, so a renumbering would be a silent ABI break.
	#[test]
	fn indices_works() {
		assert_eq!(HostCall::Gas as u64, 0);
		assert_eq!(HostCall::GrowHeap as u64, 1);
		assert_eq!(HostCall::Fetch as u64, 2);
		assert_eq!(HostCall::HistoricalLookup as u64, 7);
		assert_eq!(HostCall::Export as u64, 8);
		assert_eq!(HostCall::SetParentHeadHash as u64, 100);
		assert_eq!(HostCall::SetHead as u64, 101);
		assert_eq!(HostCall::SendUpwardMessage as u64, 102);
		assert_eq!(HostCall::ReportError as u64, 103);
	}
}
