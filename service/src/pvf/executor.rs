//! PVM executor state for recording side-effects of PVF refine invocation.

use crate::work_digest::{RefineLog, MAX_UPWARD_MESSAGES_PER_DIGEST};
use bounded_collections::{BoundedVec, ConstU32};
use codec::DecodeAll;
use jam_pvm_common::refine;
use parachain_support::types::UpwardMessage;

/// Side-effect buffer during refine invoke-PVM loop.
#[derive(Default)]
pub struct ExecutorState {
	pub umps: BoundedVec<UpwardMessage, ConstU32<MAX_UPWARD_MESSAGES_PER_DIGEST>>,
}

impl ExecutorState {
	pub fn send_upward_raw(self, handle: u64, ptr: u64, len: u64) -> Result<Self, RefineLog> {
		let buffer = refine::peek(handle, ptr, len).map_err(|_| RefineLog::ValidationFailed)?;
		let msg =
			UpwardMessage::decode_all(&mut &buffer[..]).map_err(|_| RefineLog::MalformedPayload)?;

		self.send_upward(msg)
	}

	fn send_upward(mut self, msg: UpwardMessage) -> Result<Self, RefineLog> {
		self.umps.try_push(msg).map_err(|_| RefineLog::TooManyUpwardMessages)?;

		Ok(self)
	}
}
