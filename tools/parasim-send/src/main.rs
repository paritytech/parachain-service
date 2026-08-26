//! parasim mock work-package sender.
//!
//! Prints the SCALE-encoded `ParachainCandidate` work-item payload for a fake
//! `ParachainBlockData::V1` PoV — bytes you can hand to `jamt item`. The PoV
//! carries one hand-built substrate `Header`; the `--number` argument is stamped
//! into the header's `state_root`, so sending twice with different `--number`
//! yields visibly different heads while keeping every other byte stable. Read the
//! updated para head back with `jamt service storage`.
//!
//! The dev-genesis null authorizer (empty config) makes refine fall back to
//! `FALLBACK_PARA_ID` (0), so the head is always stored under para 0 — read it
//! back at key `0x0000000000`. A real authorizer-config list (phase 3) restores
//! `--para`-selectable paras from the config.
//!
//! In a loop, this witnesses `refine`→`accumulate`→`set_storage` land on-chain:
//!
//! ```text
//! for n in 1 2 3; do
//!   payload=$(parasim-send --number "$n")
//!   jamt item <service-id> "$payload" --force-core 0
//!   sleep 2
//!   jamt service storage <service-id> 0x0000000000 --raw
//! done
//! ```

use codec::{Compact, Encode};
use parachain_service_interface::candidate::ParachainCandidate;
use parachain_service_interface::types::{ParaId, ValidationCodeHash};

/// Literal prefix of `ParachainBlockData::V1` (mirrors `parasim_service::pov`).
const VERSIONED_PREFIX: &[u8] = b"VERSIONEDPBD";
const V1_VERSION: u8 = 1;
const HASH_SIZE: usize = 32;

fn main() {
	let mut number = 1u32;
	let mut it = std::env::args().skip(1);
	while let Some(arg) = it.next() {
		let val = match it.next() {
			Some(v) => v,
			None => {
				eprintln!("{arg} requires a value");
				std::process::exit(2);
			},
		};
		match arg.as_str() {
			"--number" => number = val.parse().map_err(|_| {
				eprintln!("--number: expected a number, got {val:?}");
				std::process::exit(2)
			}).unwrap_or(1),
			_ => {
				eprintln!("unknown argument {arg}");
				std::process::exit(2)
			},
		}
	}

	let pov = fake_v1_pov(number);
	let candidate =
		ParachainCandidate { validation_code_hash: ValidationCodeHash([0u8; 32]), pov };
	let payload_hex = hex(&candidate.encode());
	// On the dev genesis every core runs the null authorizer (empty config), so
	// refine falls back to `FALLBACK_PARA_ID` (0) — the head lands at para 0. A
	// real authorizer-config list (phase 3) would select paras `--para` instead.
	let stored_para = ParaId(0);
	let key = parasim_service::para_head_key(stored_para);

	// Self-check: the parasim parser must accept what this builder produces.
	// Any drift between the two fails fast here instead of at chain runtime.
	match parasim_service::pov::decode_para_head(&candidate.pov) {
		Ok(head) => eprintln!("self-check: refine would extract {} bytes (head number {})", head.len(), number),
		Err(_) => {
			eprintln!("self-check FAILED: parasim rejects the payload this sender builds");
			std::process::exit(1);
		},
	}

	println!("# work-item payload for `jamt item <id> <this>` (head number {number})");
	println!("{payload_hex}");
	println!("# then read the head: jamt service storage <id> 0x{} --raw", hex(&key));
}

/// Build a `ParachainBlockData::V1` PoV: one block, empty extrinsics, an empty
/// `CompactProof`.
fn fake_v1_pov(number: u32) -> Vec<u8> {
	let mut pov = Vec::new();
	pov.extend_from_slice(VERSIONED_PREFIX);
	pov.push(V1_VERSION);
	Compact::from(1u32).encode_to(&mut pov); // one block
	pov.extend_from_slice(&header_bytes(number));
	Compact::from(0u32).encode_to(&mut pov); // empty Vec<OpaqueExtrinsic>
	Compact::from(0u32).encode_to(&mut pov); // CompactProof: empty encoded_nodes
	pov
}

/// Encode a substrate `Header<u32>`: parent + compact number + state_root +
/// extrinsics_root + empty `Digest`.
fn header_bytes(number: u32) -> Vec<u8> {
	let mut h = Vec::new();
	h.extend_from_slice(&[0; HASH_SIZE]); // parent_hash
	h.extend_from_slice(&Compact::from(number).encode());
	let mut state_root = [0u8; HASH_SIZE];
	state_root[..4].copy_from_slice(&number.to_le_bytes());
	h.extend_from_slice(&state_root);
	h.extend_from_slice(&[0; HASH_SIZE]); // extrinsics_root
	h.push(0); // empty Digest
	h
}

fn hex(bytes: &[u8]) -> String {
	use std::fmt::Write as _;
	let mut s = String::with_capacity(bytes.len() * 2);
	for b in bytes {
		write!(s, "{b:02x}").ok();
	}
	s
}