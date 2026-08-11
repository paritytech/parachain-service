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
	/// Import segments metadata.
	ImportSegments = 10,
	/// A specific import segment by index.
	ImportSegment = 11,

	// --- Side-effects ---
	/// Write a segment to the JAM Data Lake (e.g. outbound XCMP payloads). Returns segment index.
	Export = 12,
	/// Declare the parent head hash this candidate was built on.
	SetParentHeadHash = 13,
	/// Declare the new head data this parachain block produced.
	SetHead = 14,
	/// Signal a PVF code upgrade request.
	RequestCodeUpgrade = 15,
	/// Mediated forward of JAM's `solicit`.
	Solicit = 16,
	/// Mediated forward of JAM's `forget`.
	Forget = 17,
	/// Upsert key_value_storage entry.
	KvSet = 18,
	/// Remove key_value_storage entry.
	KvRemove = 19,
	/// Transfer balance to another JAM service.
	TransferOut = 20,
	/// Schedule a core's assign.
	AssignCore = 21,
	/// Append a chunk of upcoming validator keys.
	SetValidatorKeys = 22,
	/// Drop queued transfer buckets up to a slot.
	ConsumeTransfersUpTo = 23,
	/// Replace the Parachain Service's own service code.
	ParachainServiceUpgrade = 24,
	/// Abort the PVF with an opaque error payload.
	ReportError = 25,
	/// Upsert a parachain's head data (Coretime chain only).
	ParachainSetHead = 26,
	/// Upsert a parachain's validation code (Coretime chain only).
	ParachainSetValidationCode = 27,
	/// Remove all per-parachain state (Coretime chain only).
	ParachainCleanUp = 28,
	/// Overwrite a parachain's total state balance (Coretime chain only).
	ParachainSetStateBalance = 29,
}
