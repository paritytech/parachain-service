//! Connecting to a JAM node.

use jam_rpc_interface::JamRpcInterface;

/// Connect and spawn the connection worker.
///
/// The worker future drives the connection, request replay and block fan-out; without it every
/// request would hang forever, so it is spawned here rather than left to each caller.
pub async fn connect(url: &str) -> Result<JamRpcInterface, String> {
	let url = url.parse().map_err(|error| format!("bad --rpc URL: {error}"))?;
	let (jam, worker) = JamRpcInterface::new(vec![url])
		.await
		.map_err(|error| format!("cannot reach the JAM node: {error}"))?;
	tokio::spawn(worker);
	Ok(jam)
}
