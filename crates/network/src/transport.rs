//! Transport layer configuration
//!
//! DNS/TCP + Noise encryption + Yamux multiplexing stack for DOLI P2P.
//! Optionally composes a relay client transport for NAT traversal.

use libp2p::{core::upgrade, dns, identity::Keypair, noise, relay, tcp, yamux, PeerId, Transport};

/// 512KB yamux receive window. Sync headers are ~100KB per batch — 512KB
/// is 5x headroom. Halves per-connection RAM vs 1MB default, enabling
/// 300 nodes on 128GB (INC-I-013).
#[allow(deprecated)] // set_receive_window_size will be replaced in next yamux breaking release
fn yamux_config() -> yamux::Config {
    let mut cfg = yamux::Config::default();
    let window = std::env::var("DOLI_YAMUX_WINDOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(524_288u32);
    cfg.set_receive_window_size(window);
    cfg
}

/// Build the transport stack: DNS/TCP + Noise + Yamux, optionally composed
/// with a relay client transport for NAT traversal.
pub fn build_transport(
    keypair: &Keypair,
    relay_transport: Option<relay::client::Transport>,
) -> std::io::Result<libp2p::core::transport::Boxed<(PeerId, libp2p::core::muxing::StreamMuxerBox)>>
{
    let tcp_config = tcp::Config::default().nodelay(true);
    let tcp_transport = tcp::tokio::Transport::new(tcp_config);
    let dns_transport =
        dns::tokio::Transport::system(tcp_transport).map_err(std::io::Error::other)?;

    let noise_config =
        noise::Config::new(keypair).expect("Noise config should be valid with ed25519 keypair");

    let base = dns_transport
        .upgrade(upgrade::Version::V1)
        .authenticate(noise_config)
        .multiplex(yamux_config())
        .boxed();

    let transport = match relay_transport {
        Some(relay) => {
            let relay_boxed = relay
                .upgrade(upgrade::Version::V1)
                .authenticate(
                    noise::Config::new(keypair)
                        .expect("Noise config should be valid with ed25519 keypair"),
                )
                .multiplex(yamux_config())
                .boxed();
            base.or_transport(relay_boxed)
                .map(|either, _| match either {
                    futures::future::Either::Left(v) => v,
                    futures::future::Either::Right(v) => v,
                })
                .boxed()
        }
        None => base,
    };

    Ok(transport)
}
