//! Pathfinding for multi-hop payments (Phase 2).
//!
//! Dijkstra over the channel graph to find payment routes.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use doli_core::Amount;
use serde::{Deserialize, Serialize};

/// A node in the channel graph.
pub type NodeId = [u8; 32];

/// An edge in the channel graph (a channel between two nodes).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelEdge {
    /// Channel ID.
    pub channel_id: [u8; 32],
    /// Source node.
    pub source: NodeId,
    /// Destination node.
    pub target: NodeId,
    /// Available capacity for forwarding.
    pub capacity: Amount,
    /// Fee rate in parts per million.
    pub fee_rate_ppm: u32,
    /// Base fee in base units.
    pub base_fee: Amount,
}

impl ChannelEdge {
    /// Calculate the fee for forwarding an amount through this channel.
    pub fn fee_for_amount(&self, amount: Amount) -> Amount {
        self.base_fee + (amount * self.fee_rate_ppm as u64) / 1_000_000
    }
}

/// A route is an ordered list of hops.
#[derive(Clone, Debug)]
pub struct Route {
    pub hops: Vec<RouteHop>,
    pub total_fee: Amount,
    pub total_amount: Amount,
}

/// A single hop in a route.
#[derive(Clone, Debug)]
pub struct RouteHop {
    pub channel_id: [u8; 32],
    pub node_id: NodeId,
    pub amount_to_forward: Amount,
    pub fee: Amount,
    pub expiry_delta: u64,
}

/// Channel graph for pathfinding.
#[derive(Clone, Debug, Default)]
pub struct ChannelGraph {
    /// Adjacency list: node → outgoing edges.
    edges: HashMap<NodeId, Vec<ChannelEdge>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DijkstraState {
    cost: u64,
    node: NodeId,
}

impl Ord for DijkstraState {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.cmp(&self.cost) // min-heap
    }
}

impl PartialOrd for DijkstraState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl ChannelGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a channel edge (bidirectional).
    pub fn add_channel(&mut self, edge: ChannelEdge) {
        let reverse = ChannelEdge {
            channel_id: edge.channel_id,
            source: edge.target,
            target: edge.source,
            capacity: edge.capacity,
            fee_rate_ppm: edge.fee_rate_ppm,
            base_fee: edge.base_fee,
        };
        self.edges.entry(edge.source).or_default().push(edge);
        self.edges.entry(reverse.source).or_default().push(reverse);
    }

    /// Find a route from source to destination for the given amount.
    ///
    /// Uses Dijkstra's algorithm with fee as the cost metric.
    /// Returns None if no route exists with sufficient capacity.
    pub fn find_route(
        &self,
        source: &NodeId,
        destination: &NodeId,
        amount: Amount,
    ) -> Option<Route> {
        if source == destination {
            return Some(Route {
                hops: Vec::new(),
                total_fee: 0,
                total_amount: amount,
            });
        }

        let mut dist: HashMap<NodeId, u64> = HashMap::new();
        let mut prev: HashMap<NodeId, (NodeId, ChannelEdge)> = HashMap::new();
        let mut heap = BinaryHeap::new();

        dist.insert(*source, 0);
        heap.push(DijkstraState {
            cost: 0,
            node: *source,
        });

        while let Some(DijkstraState { cost, node }) = heap.pop() {
            if node == *destination {
                break;
            }

            if cost > *dist.get(&node).unwrap_or(&u64::MAX) {
                continue;
            }

            if let Some(edges) = self.edges.get(&node) {
                for edge in edges {
                    if edge.capacity < amount {
                        continue;
                    }

                    let fee = edge.fee_for_amount(amount);
                    let new_cost = cost + fee;

                    if new_cost < *dist.get(&edge.target).unwrap_or(&u64::MAX) {
                        dist.insert(edge.target, new_cost);
                        prev.insert(edge.target, (node, edge.clone()));
                        heap.push(DijkstraState {
                            cost: new_cost,
                            node: edge.target,
                        });
                    }
                }
            }
        }

        // Reconstruct path
        if !prev.contains_key(destination) {
            return None;
        }

        let mut hops = Vec::new();
        let mut current = *destination;
        let mut total_fee = 0;

        while current != *source {
            let (prev_node, edge) = prev.get(&current)?;
            let fee = edge.fee_for_amount(amount);
            total_fee += fee;
            hops.push(RouteHop {
                channel_id: edge.channel_id,
                node_id: current,
                amount_to_forward: amount,
                fee,
                expiry_delta: 40, // default CLTV delta
            });
            current = *prev_node;
        }

        hops.reverse();

        Some(Route {
            hops,
            total_fee,
            total_amount: amount + total_fee,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u8) -> NodeId {
        let mut n = [0u8; 32];
        n[0] = id;
        n
    }

    fn channel(id: u8, src: u8, dst: u8, capacity: u64) -> ChannelEdge {
        let mut cid = [0u8; 32];
        cid[0] = id;
        ChannelEdge {
            channel_id: cid,
            source: node(src),
            target: node(dst),
            capacity,
            fee_rate_ppm: 1000, // 0.1%
            base_fee: 10,
        }
    }

    #[test]
    fn direct_route() {
        let mut graph = ChannelGraph::new();
        graph.add_channel(channel(1, 1, 2, 100_000));

        let route = graph.find_route(&node(1), &node(2), 50_000).unwrap();
        assert_eq!(route.hops.len(), 1);
        assert!(route.total_fee > 0);
    }

    #[test]
    fn multi_hop_route() {
        let mut graph = ChannelGraph::new();
        graph.add_channel(channel(1, 1, 2, 100_000));
        graph.add_channel(channel(2, 2, 3, 100_000));

        let route = graph.find_route(&node(1), &node(3), 50_000).unwrap();
        assert_eq!(route.hops.len(), 2);
    }

    #[test]
    fn no_route_insufficient_capacity() {
        let mut graph = ChannelGraph::new();
        graph.add_channel(channel(1, 1, 2, 100));

        let route = graph.find_route(&node(1), &node(2), 50_000);
        assert!(route.is_none());
    }

    #[test]
    fn no_route_disconnected() {
        let mut graph = ChannelGraph::new();
        graph.add_channel(channel(1, 1, 2, 100_000));

        let route = graph.find_route(&node(1), &node(3), 50_000);
        assert!(route.is_none());
    }

    #[test]
    fn same_source_destination() {
        let graph = ChannelGraph::new();
        let route = graph.find_route(&node(1), &node(1), 50_000).unwrap();
        assert_eq!(route.hops.len(), 0);
        assert_eq!(route.total_fee, 0);
    }

    #[test]
    fn fee_calculation() {
        let edge = ChannelEdge {
            channel_id: [0; 32],
            source: [0; 32],
            target: [1; 32],
            capacity: 1_000_000,
            fee_rate_ppm: 1000, // 0.1%
            base_fee: 100,
        };
        // 100 base + (500_000 * 1000 / 1_000_000) = 100 + 500 = 600
        assert_eq!(edge.fee_for_amount(500_000), 600);
    }
}
