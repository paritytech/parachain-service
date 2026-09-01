// Build the frameless runtime into a PolkaVM blob with the local `tools/pvm-builder`
// (polkavm-linker 0.30, matching the polkajam host's `polkavm 0.30` VM), then expose it
// to the host build as `wasm_binary.rs` — the replacement for `substrate-wasm-builder`
// (whose polkavm-linker 0.36 encodes `unlikely` instructions the 0.30 host VM can't decode).
#[cfg(feature = "std")]
fn main() {
	let out_dir = std::env::var("OUT_DIR").expect("No OUT_DIR");
	pvm_builder::build_runtime(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));

	let blob_path = format!("{out_dir}/frameless.polkavm");
	assert!(
		std::path::Path::new(&blob_path).exists(),
		"pvm-builder should have written {blob_path}"
	);
	let wasm_binary = format!(
		r#"
pub const WASM_BINARY_PATH: Option<&str> = Some("{blob_path}");
pub const WASM_BINARY: Option<&[u8]> = Some(include_bytes!("{blob_path}"));
pub const WASM_BINARY_BLOATY: Option<&[u8]> = Some(include_bytes!("{blob_path}"));
"#
	);
	std::fs::write(format!("{out_dir}/wasm_binary.rs"), wasm_binary).expect("write wasm_binary.rs");
	println!("cargo:rerun-if-changed={}/src", env!("CARGO_MANIFEST_DIR"));
}

#[cfg(not(feature = "std"))]
fn main() {}
