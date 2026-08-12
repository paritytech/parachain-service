//! PVM executor state for recording side-effects of PVF refine invocation.

use crate::work_digest::RefineLog;
use jam_pvm_common::refine;
use parachain_service_interface::types::{UpwardMessage, UpwardMessages};

/// Side-effect buffer during refine invoke-PVM loop.
#[derive(Default)]
pub struct ExecutorState {
	pub umps: UpwardMessages,
}

impl ExecutorState {
	pub fn kv_set_raw(
		self,
		handle: u64,
		key_ptr: u64,
		key_len: u64,
		value_ptr: u64,
		value_len: u64,
	) -> Result<Self, RefineLog> {
		let key_buff = refine::peek(handle, key_ptr as u64, key_len as u64)
			.map_err(|_| RefineLog::ValidationFailed)?;
		let key = unsafe {
			core::slice::from_raw_parts(key_buff.as_ptr() as *const u8, key_len as usize)
		};
		let value_buff =
			refine::peek(handle, value_ptr, value_len).map_err(|_| RefineLog::ValidationFailed)?;
		let value = unsafe {
			core::slice::from_raw_parts(value_buff.as_ptr() as *const u8, value_len as usize)
		};

		self.kv_set(key, value)
	}

	fn kv_set(mut self, key: &[u8], value: &[u8]) -> Result<Self, RefineLog> {
		self.umps
			.try_push(UpwardMessage::SetKV { key: key.to_vec(), value: value.to_vec() })
			.map_err(|_| RefineLog::TooManyUpwardMessages)?;
		Ok(self)
	}
}
