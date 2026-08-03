//! Coretime System Pallet to exercise coretime specific host functions and hooks.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[frame::pallet]
pub mod pallet {
	use frame::prelude::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
			// FIXME: Call some Coretime-only host functions here.
			todo!()
		}
	}
}
