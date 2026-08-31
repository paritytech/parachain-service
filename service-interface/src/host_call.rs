use num_enum::{IntoPrimitive, TryFromPrimitive};

/// Host call indices as per §4.3.
#[derive(IntoPrimitive, TryFromPrimitive)]
#[repr(u64)]
pub enum HostCall {
	// --- Data Access ---
	/// Fetch a preimage from the service's own store.
	Lookup = 0,
	/// Fetch a preimage from another service's store.
	ForeignLookup = 1,
	/// Query the remaining gas budget.
	Gas = 2,
	/// Access the full encoded work package.
	WorkPackage = 3,
	/// Access the refinement context.
	WorkPackageContext = 4,
	/// Access the authorizer config blob.
	AuthConfig = 5,
	/// Access the authorization token blob.
	AuthToken = 6,
	/// Summary of all work items.
	WorkItemsSummary = 7,
	/// Summary of a specific work item by index.
	WorkItemSummary = 8,
	/// Payload of a specific work item by index.
	WorkItemPayload = 9,
	/// A specific import segment, by its index in the work item's import
	/// manifest. Indices `0 .. import_count` enumerate the segments in
	/// manifest order.
	ImportSegment = 10,

	// --- Side-effects ---
	/// Write a segment to the JAM Data Lake (e.g. outbound XCMP payloads). Returns segment index.
	Export = 11,
	/// Declare the parent head hash this candidate was built on.
	SetParentHeadHash = 12,
	/// Declare the new head data this parachain block produced.
	SetHead = 13,
	/// Append one SCALE-encoded upward message to the work digest.
	SendUpwardMessage = 14,
	/// Abort the PVF with an opaque error payload.
	ReportError = 15,
}
