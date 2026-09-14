//! Parachain-service-specific host function wrappers for PolkaVM guests.
//!
//! Indices 200-203 are the Parachain Service extensions to the JAM Gray Paper host
//! call set (§4.3). These are the child-PVM imports a PVF guest uses to communicate
//! validation results back to the service host. They sit at a different index space
//! than the JAM host calls the service itself uses (`fetch` = 2, etc. in
//! [`crate::host_call::HostCall`]) — conflating the two sets is a silent ABI break.

use alloc::vec::Vec;
use codec::{Compact, Encode};

/// The subset of the service's `UpwardMessage` ABI this runtime emits. The SCALE variant
/// index is positional, so the ordering has to match the spec's `enum UpwardMessage`.
#[derive(Encode)]
enum UpwardMessage {
	RequestCodeUpgrade { hash: [u8; 32], len: Compact<u32> },
}

#[polkavm_derive::polkavm_import]
extern "C" {
	#[polkavm_import(index = 200)]
	fn set_parent_head_hash_raw(hash_ptr: u32);
	#[polkavm_import(index = 201)]
	fn set_head_raw(ptr: u32, len: u32);
	#[polkavm_import(index = 202)]
	fn send_upward_message_raw(ptr: u32, len: u32);
	#[polkavm_import(index = 203)]
	fn report_error_raw(ptr: u32, len: u32);
}

/// Declare the parent head hash this candidate was built on (called once).
pub fn set_parent_head_hash(hash: &[u8; 32]) {
	unsafe { set_parent_head_hash_raw(hash.as_ptr() as u32) }
}

/// Declare the new head data this parachain block produced.
pub fn set_head(head: &[u8]) {
	unsafe { set_head_raw(head.as_ptr() as u32, head.len() as u32) }
}

/// Signal a PVF code upgrade request (`hash` + encoded-code length).
pub fn request_code_upgrade(hash: [u8; 32], len: u32) {
	send_upward_message(&UpwardMessage::RequestCodeUpgrade { hash, len: Compact(len) }.encode())
}

/// Append one upward message to the work digest.
fn send_upward_message(msg: &[u8]) {
	unsafe { send_upward_message_raw(msg.as_ptr() as u32, msg.len() as u32) }
}

/// Abort the PVF with an opaque error payload; never returns.
pub fn report_error(data: &[u8]) -> ! {
	unsafe { report_error_raw(data.as_ptr() as u32, data.len() as u32) }
	unreachable!("`report_error` aborts the PVF; qed")
}