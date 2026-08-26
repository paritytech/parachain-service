//! Network tests against a locally running polkajam node. Ignored by default; run with a node
//! up (`polkajam-testnet --num-ordinary-nodes 1`, RPC on 19800) via `cargo test -- --ignored`.

use futures::StreamExt;
use jam_interface::{JamChainSource, JamStateSource};
use jam_rpc_interface::JamRpcInterface;
use url::Url;

async fn connect() -> JamRpcInterface {
	let url = Url::parse("ws://127.0.0.1:19800").expect("static url");
	let (interface, worker) = JamRpcInterface::new(vec![url]).await.expect("node reachable");
	tokio::spawn(worker);
	interface
}

#[tokio::test]
#[ignore = "needs a running polkajam node on ws://127.0.0.1:19800"]
async fn chain_following_works() {
	let interface = connect().await;

	let best = interface.best_block().await.expect("best block");
	let finalized = interface.finalized_block().await.expect("finalized block");
	assert!(finalized.slot <= best.slot);

	let mut best_stream = interface.best_block_stream().await.expect("best stream");
	let next = tokio::time::timeout(std::time::Duration::from_secs(30), best_stream.next())
		.await
		.expect("a best block within 30s")
		.expect("stream open");
	assert!(next.slot >= best.slot);

	let parent = interface.parent(next.header_hash).await.expect("parent");
	assert!(parent.slot < next.slot);
	interface.state_root(parent.header_hash).await.expect("state root");
	interface.beefy_root(parent.header_hash).await.expect("beefy root");
}

#[tokio::test]
#[ignore = "needs a running polkajam node on ws://127.0.0.1:19800"]
async fn auth_queues_scan_works() {
	let interface = connect().await;
	let best = interface.best_block().await.expect("best block");
	let anchor = interface.parent(best.header_hash).await.expect("anchor");

	let queues = interface.auth_queues(anchor.header_hash).await.expect("auth queues");
	assert!(queues.iter().next().is_some());
	interface.auth_pools(anchor.header_hash).await.expect("auth pools");
}
