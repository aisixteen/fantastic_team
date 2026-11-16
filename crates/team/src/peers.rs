//! The agents a team bundle talks to, behind one trait: the kernel implements it in
//! production ([`KernelPeers`]); tests give an in-memory fake. Every send is the gated
//! `kernel.send` as the calling agent, so a peer's `callable_by` decides, never the code.

use std::sync::Arc;

/// Send `payload` to agent `to` and return its reply (errors as a value).
#[async_trait::async_trait]
pub trait Peers: Send + Sync {
    /// The reply of `to` to `payload`, as it gave it.
    async fn ask(&self, to: &str, payload: serde_json::Value) -> serde_json::Value;
}

/// [`Peers`] over a running kernel.
pub struct KernelPeers(pub Arc<fantastic_kernel::Kernel>);

#[async_trait::async_trait]
impl Peers for KernelPeers {
    async fn ask(&self, to: &str, payload: serde_json::Value) -> serde_json::Value {
        self.0
            .send(vec![fantastic_kernel::Item::new(
                fantastic_kernel::AgentId::from(to),
                payload,
            )])
            .await
            .remove(0)
            .result
    }
}
