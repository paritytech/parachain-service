//! Executable service-code preimages and the self-upgrade account fields.
use codec::Compact;
use jam_node::vm::Storage;
use parachain_service_bin::mock::MOCK_SERVICE_ID;
use parachain_service_core::upward_message::UpwardMessage;
use serde_json::Value;

use super::{codex::Codex, replay::*};

// Reserved by service_upgrade_inputs.qnt. Keep their length independent of build
// output by padding the JAM container's opaque metadata, not executable bytes.
pub const CODE_LEN: u32 = 262_144;
pub fn is_code(value: i128) -> bool {
	matches!(value, 9001 | 9002)
}

pub fn blob(value: i128, len: u32) -> Result<Vec<u8>, String> {
	if len != CODE_LEN {
		return Err("service upgrade code requires length 262144".into());
	}
	let original = parachain_service_bin::blob();
	let mut body = original.as_slice();
	// The vendored program-blob crate aliases JAM codec as `codec`. This
	// header is JAM Compact; upward-message fields above/below use SCALE.
	let metadata = <jam_codec::Compact<u32> as jam_codec::Decode>::decode(&mut body)
		.map_err(|_| "invalid service metadata length")?
		.0;
	let body = body.get(metadata as usize..).ok_or("truncated service metadata")?;
	let (mut result, metadata_len) = (1..=5)
		.find_map(|prefix_len| {
			let metadata_len = (len as usize).checked_sub(body.len() + prefix_len)?;
			let prefix = jam_codec::Encode::encode(&jam_codec::Compact(metadata_len as u32));
			(prefix.len() == prefix_len && metadata_len >= 8).then_some((prefix, metadata_len))
		})
		.ok_or("service exceeds replay code length")?;
	let prefix_len = result.len();
	result.extend_from_slice(&(value as u64).to_le_bytes());
	result.resize(prefix_len + metadata_len, 0);
	result.extend_from_slice(body);
	Ok(result)
}

pub fn message(value: &Value, codex: &mut Codex) -> Result<UpwardMessage, String> {
	let hash = integer(field(field(value, "codeHash")?, "hashBytes")?)?;
	let len = bounded_integer::<u32>(field(value, "len")?, "service code length")?;
	// A claimed length is not the preimage's identity. In particular, a wrong
	// length must reach Accumulate's availability check with the same code hash.
	let code_hash = if is_code(hash) {
		codex.hash(hash, CODE_LEN)?
	} else if let Some(known) = codex.known_hash(hash) {
		known
	} else {
		codex.hash(hash, len)?
	};
	Ok(UpwardMessage::UpgradeService {
		code_hash,
		len: Compact(len),
		min_acc_gas: bounded_integer(field(value, "minAccGas")?, "minimum accumulate gas")?,
		min_memo_gas: bounded_integer(field(value, "minMemoGas")?, "minimum memo gas")?,
	})
}

pub fn seed(storage: &mut Storage, frame: &Value) -> Result<(), String> {
	let Some(minimum) = frame.get("replayMinAccGas") else {
		return Ok(());
	};
	let mut service = storage.service(MOCK_SERVICE_ID).ok_or("missing service")?;
	service.min_item_gas = bounded_integer(minimum, "minimum accumulate gas")?;
	service.min_memo_gas = bounded_integer(
		field(field(field(frame, "svc")?, "jamAccount")?, "minMemoGas")?,
		"minimum memo gas",
	)?;
	storage.set_service(MOCK_SERVICE_ID, &service);
	Ok(())
}

pub fn compare(storage: &Storage, frame: &Value, index: usize) -> Result<(), String> {
	let Some(minimum) = frame.get("replayMinAccGas") else {
		return Ok(());
	};
	let service = storage.service(MOCK_SERVICE_ID).ok_or("missing service")?;
	let svc = field(frame, "svc")?;
	let min_acc = bounded_integer::<u64>(minimum, "minimum accumulate gas")?;
	let min_memo = bounded_integer::<u64>(
		field(field(svc, "jamAccount")?, "minMemoGas")?,
		"minimum memo gas",
	)?;
	if service.min_item_gas != min_acc || service.min_memo_gas != min_memo {
		return Err(format!("frame {index}: service upgrade gas settings differ; expected=({min_acc}, {min_memo}), actual=({}, {})", service.min_item_gas, service.min_memo_gas));
	}
	// Initial model code zero has a placeholder length of zero. Every upgrade
	// names a real preimage and must agree with the host's installed code length.
	if integer(field(field(svc, "serviceCodeHash")?, "hashBytes")?)? != 0 {
		let len = bounded_integer::<u32>(field(svc, "serviceCodeLen")?, "service code length")?;
		if storage.lookup(MOCK_SERVICE_ID, service.code_hash.0).map(|b| b.len()) !=
			Some(len as usize)
		{
			return Err(format!("frame {index}: serviceCodeLen differs from installed preimage"));
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn fixture() -> Value {
		serde_json::from_str(include_str!(
			"../../fixtures/quint/service_upgrades/activation_works.itf.json"
		))
		.unwrap()
	}
	fn n(value: i128) -> Value {
		json!({"#bigint": value.to_string()})
	}

	#[test]
	fn account_and_code_mutations_errors() {
		let trace = fixture();
		let index = trace["states"]
			.as_array()
			.unwrap()
			.iter()
			.position(|s| integer(&s["svc"]["serviceCodeHash"]["hashBytes"]).unwrap() == 9001)
			.unwrap();
		for (path, expected) in [
			("/replayMinAccGas", "gas settings"),
			("/svc/jamAccount/minMemoGas", "gas settings"),
			("/svc/serviceCodeLen", "serviceCodeLen"),
			("/svc/serviceCodeHash/hashBytes", "serviceCodeHash"),
		] {
			let mut bad = trace.clone();
			let value = bad["states"][index].pointer_mut(path).unwrap();
			*value = n(integer(value).unwrap() + 1);
			assert!(document_trace(&bad).unwrap_err().contains(expected), "{path}");
		}
	}

	#[test]
	fn wrong_length_preserves_hash_works() {
		let mut codex = Codex::default();
		let value = json!({"codeHash":{"hashBytes":n(9001)},"len":n(CODE_LEN as i128 + 1),
			"minAccGas":n(1),"minMemoGas":n(2)});
		let UpwardMessage::UpgradeService { code_hash, len, .. } =
			message(&value, &mut codex).unwrap()
		else {
			unreachable!()
		};
		assert_eq!(code_hash, jam_std_common::hash_raw(&blob(9001, CODE_LEN).unwrap()));
		assert_eq!(len.0, CODE_LEN + 1);
		assert_ne!(code_hash, jam_std_common::hash_raw(&blob(9002, CODE_LEN).unwrap()));
	}

	#[test]
	fn malformed_upgrade_values_errors() {
		let original = json!({"codeHash":{"hashBytes":n(9001)},"len":n(CODE_LEN.into()),
			"minAccGas":n(1),"minMemoGas":n(2)});
		for (path, value) in [
			("/len", -1),
			("/len", 1i128 << 32),
			("/minAccGas", -1),
			("/minAccGas", 1i128 << 64),
			("/minMemoGas", -1),
			("/minMemoGas", 1i128 << 64),
		] {
			let mut bad = original.clone();
			*bad.pointer_mut(path).unwrap() = n(value);
			assert!(message(&bad, &mut Codex::default()).is_err(), "{path}");
		}
	}

	#[test]
	fn missing_gas_oracle_errors() {
		let mut trace = fixture();
		trace["states"][1].as_object_mut().unwrap().remove("replayMinAccGas");
		assert!(document_trace(&trace).unwrap_err().contains("require replayMinAccGas"));
		for state in trace["states"].as_array_mut().unwrap() {
			state.as_object_mut().unwrap().remove("replayMinAccGas");
		}
		assert!(document_trace(&trace).unwrap_err().contains("require replayMinAccGas"));
	}
}
