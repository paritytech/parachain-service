//! Validator-key input and output mapping. Abstract keys occupy the first eight
//! bytes (little endian) of the 336-byte JAM key record; the rest is zero.
use jam_node::vm::StateMutations;
use parachain_service_core::types::ValidatorKey;
use serde_json::Value;

use super::replay::*;

pub fn keys(value: &Value) -> Result<Vec<ValidatorKey>, String> {
	value
		.as_array()
		.ok_or("validator keys must be a list")?
		.iter()
		.map(|v| {
			let n = bounded_integer::<u64>(v, "validator key")?;
			let mut key = [0; 336];
			key[..8].copy_from_slice(&n.to_le_bytes());
			Ok(key)
		})
		.collect()
}

/// Privilege is an invocation input, independent of the expected designation.
/// Legacy traces omit the field and retain the mock's default privilege.
pub fn validate(states: &[Value]) -> Result<(), String> {
	let enabled = states.first().is_some_and(|s| s.get("replayCanDesignate").is_some());
	for state in states {
		if state.get("replayCanDesignate").is_some() != enabled {
			return Err("replayCanDesignate must be present in every frame".into());
		}
		if let Some(value) = state.get("replayCanDesignate") {
			boolean(value)?;
		}
	}
	Ok(())
}

pub fn privileges(
	frame: &Value,
	privileges: &mut jam_std_common::Privileges,
) -> Result<(), String> {
	if let Some(value) = frame.get("replayCanDesignate") {
		privileges.designate =
			if boolean(value)? { parachain_service_bin::mock::MOCK_SERVICE_ID } else { 99 };
	}
	Ok(())
}

pub fn compare(
	previous: &Value,
	current: &Value,
	mutations: &StateMutations,
	frame: usize,
) -> Result<(), String> {
	let before = keys(field(previous, "jamStagingSet")?)?;
	let after = keys(field(current, "jamStagingSet")?)?;
	// An explicit model effect distinguishes no call from re-designating the
	// same set. Historical fixtures without validator messages have no effect.
	let expected = current.get("replayDesignate").map(keys).transpose()?.unwrap_or_default();
	let expected_staging = if expected.is_empty() { &before } else { &expected };
	if &after != expected_staging {
		return Err(format!("frame {frame}: jamStagingSet differs from replayDesignate"));
	}
	let actual = mutations.keys.as_ref().map(|set| {
		set.iter()
			.map(|key| {
				let bytes = jam_types::Encode::encode(key);
				<[u8; 336]>::try_from(bytes.as_slice()).expect("fixed JAM validator key size")
			})
			.collect::<Vec<_>>()
	});
	let expected = (!expected.is_empty()).then_some(expected);
	if actual != expected {
		return Err(format!("frame {frame}: JAM designation differs from replayDesignate"));
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn list(values: &[u64]) -> Value {
		json!(values.iter().map(|v| json!({"#bigint": v.to_string()})).collect::<Vec<_>>())
	}

	fn frame(values: &[u64], effect: &[u64]) -> Value {
		json!({"jamStagingSet": list(values), "replayDesignate": list(effect)})
	}

	fn mutation(values: &[u64]) -> StateMutations {
		let mut mutations = StateMutations::new(0);
		mutations.keys = Some(
			keys(&list(values))
				.unwrap()
				.iter()
				.map(|raw| jam_types::Decode::decode(&mut &raw[..]).unwrap())
				.collect::<Vec<jam_std_common::ValKeyset>>()
				.try_into()
				.unwrap(),
		);
		mutations
	}

	#[test]
	fn same_set_designation_works() {
		let values = [0, 1, 2, 3, 4, u64::MAX];
		let before = frame(&values, &[]);
		let after = frame(&values, &values);
		compare(&before, &after, &mutation(&values), 1).unwrap();
		compare(&before, &before, &StateMutations::new(0), 1).unwrap();
	}

	#[test]
	fn missing_or_extra_designation_errors() {
		let values = [0, 1, 2, 3, 4, 5];
		let before = frame(&values, &[]);
		let after = frame(&values, &values);
		assert!(compare(&before, &after, &StateMutations::new(0), 1).is_err());
		assert!(compare(&before, &before, &mutation(&values), 1).is_err());
	}

	#[test]
	fn designation_contents_errors() {
		let before = frame(&[], &[]);
		let after = frame(&[0, 1, 2, 3, 4, 5], &[0, 1, 2, 3, 4, 5]);
		for values in [vec![1, 0, 2, 3, 4, 5], vec![0, 1, 2, 3, 4, 6], vec![0; 9]] {
			assert!(compare(&before, &after, &mutation(&values), 1).is_err());
		}
		let mut host = mutation(&[0, 1, 2, 3, 4, 5]);
		// Corrupt a byte outside the abstract integer prefix as well.
		let mut raw = keys(&list(&[0, 1, 2, 3, 4, 5])).unwrap();
		raw[0][335] = 1;
		host.keys = Some(
			raw.iter()
				.map(|raw| jam_types::Decode::decode(&mut &raw[..]).unwrap())
				.collect::<Vec<jam_std_common::ValKeyset>>()
				.try_into()
				.unwrap(),
		);
		assert!(compare(&before, &after, &host, 1).is_err());
	}

	#[test]
	fn invalid_key_integer_errors() {
		for value in ["-1", "18446744073709551616"] {
			assert!(keys(&json!([{"#bigint": value}])).unwrap_err().contains("validator key"));
		}
	}
}
