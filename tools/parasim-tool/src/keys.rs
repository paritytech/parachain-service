//! `display-key`: read a service-storage entry and decode it.
//!
//! Only the para head is understood today, hence `display-key parahead <para-id>`. Naming the
//! subject leaves room for other entries as the service grows, and it is also a reminder that the
//! bytes are not self-describing: the caller has to say what they expect.

use codec::Decode as _;
use jam_interface::{JamChainSource, JamStateSource, ServiceId};
use jam_rpc_interface::JamRpcInterface;
use parachain_service_interface::types::ParaId;

use crate::{
	format::{hex, parse_header_hash},
	header,
};

/// Print the para head stored for `para` by `service`, decoded unless `raw`.
pub async fn display_parahead(
	jam: &JamRpcInterface,
	service: ServiceId,
	para: ParaId,
	at: Option<String>,
	raw: bool,
) -> Result<(), String> {
	let at = match at {
		Some(hash) => parse_header_hash(&hash)?,
		None => jam.best_block().await.map_err(|e| format!("best block: {e}"))?.header_hash,
	};

	let service_local_key = parasim_service::para_head_key(para);
	let state_key = jam_state_helpers::service_value_state_key(service, &service_local_key);
	println!("block       0x{}", hex(&*at));
	println!("service     {service}");
	println!("para        {}", para.0);
	println!("service key 0x{}  (this is what set_storage/serviceValue take)", hex(&service_local_key));
	println!("state key   0x{}  (this is what stateProof/stateValue take)", hex(&state_key));

	let Some(stored) = jam
		.service_value(at, service, &service_local_key)
		.await
		.map_err(|e| format!("reading the para head: {e}"))?
	else {
		println!("\nno entry: para {} has no head at this block", para.0);
		return Ok(());
	};

	println!("\nParaInfo    {} bytes", stored.len());
	if raw {
		println!("0x{}", hex(&stored));
		return Ok(());
	}
	print_para_info(&stored)
}

/// Decode and print a stored `ParaInfo`, then the substrate header inside its `head_data`.
fn print_para_info(stored: &[u8]) -> Result<(), String> {
	let info = parasim_service::ParaInfoLite::decode(&mut &stored[..])
		.map_err(|e| format!("not a decodable ParaInfo: {e} (try --raw)"))?;
	let head = info.head_data.into_inner();

	println!("  head_data           {} bytes", head.len());
	println!("  validation_code     {:?}", info.validation_code);
	println!("  pending_upgrade     {:?}", info.pending_upgrade);
	println!("  total_state_balance {}", info.total_state_balance);
	println!("  used_state_balance  {}", info.used_state_balance);
	println!("  is_deregistering    {}", info.is_deregistering);

	println!("\nhead (substrate header)");
	println!("  hash        0x{}", hex(&jam_state_helpers::blake2_256(&head)));
	match header::decode(&head) {
		Ok(header) => {
			println!("  parent_hash 0x{}", hex(&header.parent_hash));
			println!("  number      {}", header.number);
			println!("  state_root  0x{}", hex(&header.state_root));
		},
		Err(error) => println!("  (undecodable: {error})"),
	}
	println!("  encoded     0x{}", hex(&head));
	Ok(())
}
