//! Transport layer configuration
//!
//! TCP + Noise encryption + Yamux multiplexing stack for DOLI P2P.

use libp2p::{core::upgrade, identity::Keypair, noise, tcp, yamux, PeerId, Transport};

/// Build the transport stack: TCP + Noise + Yamux
pub fn build_transport(
    keypair: &Keypair,
) -> std::io::Result<libp2p::core::transport::Boxed<(PeerId, libp2p::core::muxing::StreamMuxerBox)>>
{
    let tcp_config = tcp::Config::default().nodelay(true);
    let tcp_transport = tcp::tokio::Transport::new(tcp_config);

    let noise_config =
        noise::Config::new(keypair).expect("Noise config should be valid with ed25519 keypair");

    let yamux_config = yamux::Config::default();

    let transport = tcp_transport
        .upgrade(upgrade::Version::V1)
        .authenticate(noise_config)
        .multiplex(yamux_config)
        .boxed();

    Ok(transport)
}
