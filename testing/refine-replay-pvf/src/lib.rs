//! Test-only PVF executing an ordered script supplied as extrinsic zero.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
use alloc::vec::Vec;
use codec::{Decode, Encode};
// Supplies the guest allocator and panic handler.
use parachain_service_core as _;

#[derive(Debug, Encode, Decode)]
pub enum Call {
	Parent([u8; 32]),
	Head(Vec<u8>),
	Send(Vec<u8>),
	Error(Vec<u8>),
	Panic,
	/// Consume every finite child gas budget without returning.
	ExhaustGas,
}

#[cfg(target_arch = "riscv64")]
#[polkavm_derive::polkavm_export]
extern "C" fn jam_validate_block() {
	use codec::DecodeAll;
	#[polkavm_derive::polkavm_import]
	extern "C" {
		#[polkavm_import(index = 2)]
		fn fetch(buffer: u32, offset: u64, len: u64, kind: u64, a: u64, b: u64) -> u64;
		#[polkavm_import(index = 200)]
		fn parent(ptr: u32);
		#[polkavm_import(index = 201)]
		fn head(ptr: u32, len: u32);
		#[polkavm_import(index = 202)]
		fn send(ptr: u32, len: u32);
		#[polkavm_import(index = 203)]
		fn error(ptr: u32, len: u32);
	}
	let len = unsafe { fetch(0, 0, 0, 4, 0, 0) };
	assert!(len < 4 * 1024 * 1024, "missing or oversized script");
	let mut bytes = alloc::vec![0; len as usize];
	assert_eq!(unsafe { fetch(bytes.as_mut_ptr() as u32, 0, len, 4, 0, 0) }, len);
	let calls = Vec::<Call>::decode_all(&mut &bytes[..]).expect("invalid script");
	for call in calls {
		unsafe {
			match call {
				Call::Parent(data) => parent(data.as_ptr() as u32),
				Call::Head(data) => head(data.as_ptr() as u32, data.len() as u32),
				Call::Send(data) => send(data.as_ptr() as u32, data.len() as u32),
				Call::Error(data) => error(data.as_ptr() as u32, data.len() as u32),
				Call::ExhaustGas => loop {
					core::hint::black_box(0u32);
				},
				Call::Panic => panic!("scripted PVF panic"),
			}
		}
	}
}
