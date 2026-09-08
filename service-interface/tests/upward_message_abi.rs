//! The upward-message SCALE ABI, pinned against the real parachain service.
//!
//! `UpwardMessage` is the vocabulary this repo's sudo lane speaks so that the same bytes will one
//! day be handed to the real service unchanged. Nothing in this repo runs that service, so a
//! reordered or inserted variant would not break any build here — it would only make our
//! `AssignCore` arrive over there as some other message. Two pins stand in for the missing
//! compiler:
//!
//! - [`variant_indices_are_pinned_works`] names the SCALE discriminant of every variant, so
//!   moving one is a test failure rather than a wire change nobody sees.
//! - [`matches_the_real_service_source_works`] compares the enum's source text with the real
//!   checkout's, when one is next to this one. That is the only check that catches the real
//!   service moving *first*, which is what happened before this file existed.

use parachain_service_interface::{
	types::{Hash, ParaId, ServiceId, ValidationCodeHash},
	upward_message::{CreateServiceArgs, Target, TransferOutArgs, UpwardMessage},
};

use codec::Encode;
use std::path::PathBuf;

const SERVICE: ServiceId = 5;
const PARA: ParaId = ParaId(1);
const HASH: Hash = [0xab; 32];
const CODE_HASH: ValidationCodeHash = ValidationCodeHash(HASH);

/// Every variant, paired with the SCALE discriminant it must encode to.
///
/// Written out by hand rather than derived from declaration order: a list built by enumerating
/// the variants would agree with any order at all, which is exactly the failure being guarded
/// against.
fn pinned_variants() -> Vec<(u8, UpwardMessage)> {
	vec![
		(0, UpwardMessage::RequestCodeUpgrade { hash: CODE_HASH, len: 7.into() }),
		(1, UpwardMessage::Solicit { target: Target::Parachain(PARA), hash: HASH, len: 7.into() }),
		(2, UpwardMessage::EjectService { service: SERVICE }),
		(3, UpwardMessage::SetServiceSupervisor { service: SERVICE, new_supervisor: SERVICE }),
		(
			4,
			UpwardMessage::CreateService(CreateServiceArgs {
				code_hash: HASH,
				len: 7.into(),
				min_item_gas: 1,
				min_memo_gas: 2,
				id: 3.into(),
				desired_id: None,
				source_supervisor_balance: false,
				new_supervisor_balance: false,
			}),
		),
		(5, UpwardMessage::Forget { target: Target::Parachain(PARA), hash: HASH, len: 7.into() }),
		(6, UpwardMessage::RemoveServiceStorage { service: SERVICE, key: vec![1, 2] }),
		(7, UpwardMessage::SetKV { key: vec![1, 2], value: vec![3] }),
		(8, UpwardMessage::RemoveKV { para_id: PARA, key: vec![1, 2] }),
		(
			9,
			UpwardMessage::TransferOut(TransferOutArgs {
				source: None,
				dest: SERVICE,
				amount: 1.into(),
				id: 2.into(),
				source_supervisor_balance: false,
				dest_supervisor_balance: false,
				deferred: None,
			}),
		),
		(
			10,
			UpwardMessage::AssignCore {
				core: 1,
				queue: vec![HASH],
				new_assigner: None,
				jam_slot: 9,
			},
		),
		(11, UpwardMessage::SetValidatorKeys { keys: vec![], is_last: true }),
		(12, UpwardMessage::CleanUpBucketsUpTo(4)),
		(
			13,
			UpwardMessage::UpgradeService {
				code_hash: HASH,
				len: 7.into(),
				min_acc_gas: 1,
				min_memo_gas: 2,
			},
		),
		(
			14,
			UpwardMessage::ParachainSetHead {
				para_id: PARA,
				new_head: vec![1, 2].try_into().expect("two bytes fit; qed"),
			},
		),
		(
			15,
			UpwardMessage::ParachainSetValidationCode {
				para_id: PARA,
				new_validation_code_hash: CODE_HASH,
				new_validation_code_len: 7.into(),
			},
		),
		(16, UpwardMessage::ParachainCleanUp(PARA)),
		(17, UpwardMessage::ParachainSetStateBalance { para_id: PARA, new_total: 8.into() }),
	]
}

#[test]
fn variant_indices_are_pinned_works() {
	for (index, message) in pinned_variants() {
		assert_eq!(
			message.encode()[0],
			index,
			"{message:?} moved to discriminant {}; the real service still reads {index}",
			message.encode()[0]
		);
	}
}

/// The messages the sudo lane actually sends, byte for byte.
///
/// `AssignCore` is the one that matters: everything this repo submits is an assign, and its
/// fields happen to be identical in both repos, so nothing but the leading discriminant would
/// ever reveal a mismatch. The queue is two hashes rather than the full 80 the tool sends, to keep
/// the pin readable — the length prefix is the part under test.
#[test]
fn assign_core_bytes_are_pinned_works() {
	let assign = UpwardMessage::AssignCore {
		core: 1,
		queue: vec![HASH, [0xcd; 32]],
		new_assigner: None,
		jam_slot: 9,
	};
	let expected = concat!(
		"0a",       // discriminant: AssignCore
		"0100",     // core: u16 = 1
		"08",       // queue: Compact length 2
		"abababababababababababababababababababababababababababababababab",
		"cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
		"00",       // new_assigner: None
		"09000000", // jam_slot: u32 = 9
	);
	assert_eq!(hex(&assign.encode()), expected);
}

fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The real checkout, if it is a sibling of this one or named by `PARACHAIN_SERVICE_MAIN`.
fn real_upward_message_source() -> Option<String> {
	let path = match std::env::var_os("PARACHAIN_SERVICE_MAIN") {
		Some(root) => PathBuf::from(root),
		None => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
			.join("../../parachain-service-main")
			.canonicalize()
			.ok()?,
	}
	.join("service-interface/src/upward_message.rs");
	std::fs::read_to_string(path).ok()
}

/// The `pub enum UpwardMessage { … }` block, which is where the wire order lives.
fn enum_block(source: &str) -> &str {
	let start = source.find("pub enum UpwardMessage {").expect("the enum is declared; qed");
	let end = source[start..].find("\n}\n").expect("the declaration is closed; qed");
	&source[start..start + end]
}

/// Our copy is the real service's file, so the enum's source text must match it exactly.
///
/// Skipped rather than failed when the real checkout is not there: this repo has to build offline
/// and in CI, where it is not. The byte pins above are what keeps that case honest.
#[test]
fn matches_the_real_service_source_works() {
	let Some(real) = real_upward_message_source() else {
		eprintln!(
			"no parachain-service checkout next to this one; \
			 set PARACHAIN_SERVICE_MAIN to check the sync"
		);
		return;
	};
	let ours = include_str!("../src/upward_message.rs");
	// Line by line, because the whole block printed twice says nothing about where it drifted.
	let mut theirs = enum_block(&real).lines();
	for (number, ours) in enum_block(ours).lines().enumerate() {
		assert_eq!(
			Some(ours),
			theirs.next(),
			"`UpwardMessage` has drifted from the real service at enum line {number}; re-sync \
			 the whole file rather than patching one variant, and re-derive the byte pins above"
		);
	}
	assert_eq!(theirs.next(), None, "the real service's `UpwardMessage` has grown new variants");
}
