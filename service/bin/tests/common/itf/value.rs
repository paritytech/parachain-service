use std::collections::BTreeMap;

use serde_json::Value;

/// A strict representation of the values emitted by Quint's ITF writer.
#[derive(Clone, Debug, PartialEq)]
pub enum ItfValue {
	Int(i128),
	Bool(bool),
	Str(String),
	List(Vec<Self>),
	Set(Vec<Self>),
	Map(Vec<(Self, Self)>),
	Tup(Vec<Self>),
	Variant { tag: String, value: Box<Self> },
	Record(BTreeMap<String, Self>),
}

impl TryFrom<&Value> for ItfValue {
	type Error = String;

	fn try_from(value: &Value) -> Result<Self, Self::Error> {
		match value {
			Value::Bool(value) => Ok(Self::Bool(*value)),
			Value::String(value) => Ok(Self::Str(value.clone())),
			Value::Array(values) => {
				values.iter().map(Self::try_from).collect::<Result<_, _>>().map(Self::List)
			},
			Value::Object(object) => {
				if let Some(value) = object.get("#bigint") {
					ensure_only_key(object, "#bigint")?;
					let value = value.as_str().ok_or("#bigint must contain a string")?;
					return value
						.parse()
						.map(Self::Int)
						.map_err(|_| format!("invalid #bigint: {value}"));
				}
				if let Some(values) = object.get("#set") {
					ensure_only_key(object, "#set")?;
					return parse_array(values, "#set").map(Self::Set);
				}
				if let Some(values) = object.get("#tup") {
					ensure_only_key(object, "#tup")?;
					return parse_array(values, "#tup").map(Self::Tup);
				}
				if let Some(entries) = object.get("#map") {
					ensure_only_key(object, "#map")?;
					let entries = entries.as_array().ok_or("#map must contain an array")?;
					return entries
						.iter()
						.map(|entry| {
							let pair = entry.as_array().ok_or("#map entry must be an array")?;
							if pair.len() != 2 {
								return Err(format!(
									"#map entry must have 2 values, got {}",
									pair.len()
								));
							}
							Ok((Self::try_from(&pair[0])?, Self::try_from(&pair[1])?))
						})
						.collect::<Result<_, _>>()
						.map(Self::Map);
				}
				if object.contains_key("tag") || object.contains_key("value") {
					if object.len() != 2 ||
						!object.contains_key("tag") ||
						!object.contains_key("value")
					{
						return Err("variant must contain exactly 'tag' and 'value'".into());
					}
					let tag = object["tag"].as_str().ok_or("variant tag must be a string")?.into();
					let value = Box::new(Self::try_from(&object["value"])?);
					return Ok(Self::Variant { tag, value });
				}

				object
					.iter()
					.map(|(key, value)| Ok((key.clone(), Self::try_from(value)?)))
					.collect::<Result<_, _>>()
					.map(Self::Record)
			},
			Value::Null => Err("ITF null is not a Quint value".into()),
			Value::Number(_) => Err("ITF integers must use the #bigint encoding".into()),
		}
	}
}

impl ItfValue {
	pub fn int(&self) -> Result<i128, String> {
		match self {
			Self::Int(value) => Ok(*value),
			other => Err(format!("expected int, got {other:?}")),
		}
	}

	pub fn variant(&self, expected_tag: &str) -> Result<&Self, String> {
		match self {
			Self::Variant { tag, value } if tag == expected_tag => Ok(value),
			Self::Variant { tag, .. } => Err(format!("expected variant {expected_tag}, got {tag}")),
			other => Err(format!("expected variant {expected_tag}, got {other:?}")),
		}
	}

	pub fn field(&self, name: &str) -> Result<&Self, String> {
		match self {
			Self::Record(fields) => fields.get(name).ok_or_else(|| format!("missing field {name}")),
			other => Err(format!("expected record containing {name}, got {other:?}")),
		}
	}
}

fn ensure_only_key(object: &serde_json::Map<String, Value>, key: &str) -> Result<(), String> {
	if object.len() == 1 {
		Ok(())
	} else {
		Err(format!("{key} wrapper must not contain other fields"))
	}
}

fn parse_array(value: &Value, kind: &str) -> Result<Vec<ItfValue>, String> {
	value
		.as_array()
		.ok_or_else(|| format!("{kind} must contain an array"))?
		.iter()
		.map(ItfValue::try_from)
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn all_itf_shapes_work() {
		let json = serde_json::json!({
			"int": {"#bigint": "42"},
			"bool": true,
			"str": "x",
			"list": [{"#bigint": "1"}],
			"set": {"#set": [{"#bigint": "2"}]},
			"map": {"#map": [[{"#bigint": "3"}, false]]},
			"tup": {"#tup": []},
			"option": {"tag": "Some", "value": {"#bigint": "4"}}
		});
		let value = ItfValue::try_from(&json).unwrap();
		assert_eq!(value.field("int").unwrap().int().unwrap(), 42);
		assert_eq!(value.field("option").unwrap().variant("Some").unwrap().int().unwrap(), 4);
	}

	#[test]
	fn bare_json_number_errors() {
		let error = ItfValue::try_from(&serde_json::json!(42)).unwrap_err();
		assert!(error.contains("#bigint"));
	}

	#[test]
	fn malformed_map_errors() {
		let error = ItfValue::try_from(&serde_json::json!({"#map": [[true]]})).unwrap_err();
		assert!(error.contains("2 values"));
	}
}
