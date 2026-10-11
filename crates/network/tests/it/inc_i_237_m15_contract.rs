//! INC-I-237 M1.5 — structural contract for R1 (two lanes behind a sync sink)
//! and R3 (no per-item spawn on the node->swarm broadcast paths).
//!
//! OUTPUT CONTRACT: source shape only (no runtime output).
//!   network: service/{event_sink, event_lanes, swarm_loop, behaviour_events,
//!     swarm_events, gossip_intake, command_handling, mod}.rs, non-test part
//!     (above the first #[cfg(test)]).
//!   node: bins/node/src/node/{startup, rpc_broadcast, network_events}.rs.
//! PATHS: P1 sink file tokens; P2 receiver file tokens; P3 each swarm-side file x
//!   each forbidden token; P4 handle_command body; P5 NetworkService fields;
//!   P6 each RPC broadcast wiring call; P7 each producer-gossip handler body.
//! INPUT PARTITIONS: tokens {async fn, .await, Sender accessor/return/pub field,
//!   Sender<NetworkEvent>, .send(, .try_send(, tokio::spawn, try_send(, EventLanes}.
//! MATRIX: shape x P1-P7 — all asserted.

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Cut at the first inline `#[cfg(test)]` block; `#[cfg(test)] mod x;` lines are skipped.
fn non_test(all: String) -> String {
    let mut from = 0;
    while let Some(i) = all[from..].find("#[cfg(test)]") {
        let at = from + i;
        let next = all[at + "#[cfg(test)]".len()..]
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        if next.ends_with('{') {
            return all[..at].to_string();
        }
        from = at + 1;
    }
    all
}

fn service(rel: &str) -> String {
    non_test(read(&format!(
        "{}/src/service/{rel}",
        env!("CARGO_MANIFEST_DIR")
    )))
}

fn node(rel: &str) -> String {
    non_test(read(&format!(
        "{}/../../bins/node/src/node/{rel}",
        env!("CARGO_MANIFEST_DIR")
    )))
}

fn code(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn hits(src: &str, token: &str) -> Vec<String> {
    code(src)
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains(token))
        .map(|(n, l)| format!("{}: {}", n + 1, l.trim()))
        .collect()
}

/// Text from `open` (a `(` or `{` at that index) to its matching close.
fn balanced(src: &str, open: usize) -> &str {
    let (o, c) = match src.as_bytes()[open] {
        b'(' => ('(', ')'),
        b'{' => ('{', '}'),
        x => panic!("not an opener: {}", x as char),
    };
    let mut depth = 0usize;
    for (i, ch) in src[open..].char_indices() {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return &src[open..=open + i];
            }
        }
    }
    panic!("unbalanced from {open}")
}

fn fn_body(src: &str, name: &str) -> String {
    let src = code(src);
    let at = src
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no fn {name}"));
    let open = at + src[at..].find('{').expect("fn body");
    balanced(&src, open).to_string()
}

fn assert_absent(what: &str, src: &str, tokens: &[&str]) {
    for t in tokens {
        let h = hits(src, t);
        assert!(h.is_empty(), "{what} contains `{t}`:\n{}", h.join("\n"));
    }
}

fn assert_no_sender_exposure(file: &str, src: &str) {
    for (n, l) in code(src).lines().enumerate() {
        let l = l.trim();
        if l.contains("fn ") {
            if let Some((_, ret)) = l.split_once("->") {
                assert!(
                    !ret.contains("Sender"),
                    "{file}:{} returns a Sender: {l}",
                    n + 1
                );
            }
        }
        let public = l.starts_with("pub ") || l.starts_with("pub(");
        assert!(
            !(public && l.contains("Sender<") && !l.contains("fn ")),
            "{file}:{} exposes a Sender field: {l}",
            n + 1
        );
    }
}

// REQ-GRV-006 / F-6 — Decision: a failure means the sink side of the two lanes regained an awaitable path or a Sender handle, or the receiver was merged into the sink file, so the swarm can park on the node again (cycle C1).
#[test]
fn req_grv_006_sink_file_stays_sync_and_receiver_lives_in_event_lanes() {
    let sink = service("event_sink.rs");
    assert_absent(
        "event_sink.rs",
        &sink,
        &["async ", "async{", ".await", "fn sender", ".reserve()"],
    );
    assert_no_sender_exposure("event_sink.rs", &sink);
    for f in [
        "fn emit(",
        "fn emit_block(",
        "fn try_flush(",
        "fn stats(",
        "fn event_lanes(",
        "fn lane_of(",
        "fn event_bytes(",
    ] {
        assert!(code(&sink).contains(f), "event_sink.rs lacks `{f}`");
    }
    let lanes = service("event_lanes.rs");
    assert_absent("event_lanes.rs", &lanes, &["impl EventSink", "fn sender"]);
    assert_no_sender_exposure("event_lanes.rs", &lanes);
    for f in ["struct EventLanes", "fn recv(", "fn try_recv("] {
        assert!(code(&lanes).contains(f), "event_lanes.rs lacks `{f}`");
    }
}

// REQ-GRV-006 / F-6 — Decision: a failure means a swarm-side file names the raw event Sender or sends on a channel directly, so the await-cycle is again possible by convention only.
#[test]
fn req_grv_006_swarm_side_files_never_name_or_send_on_the_event_channel() {
    for file in [
        "swarm_loop.rs",
        "behaviour_events.rs",
        "swarm_events.rs",
        "gossip_intake.rs",
    ] {
        assert_absent(
            file,
            &service(file),
            &["Sender<NetworkEvent>", ".send(", ".try_send("],
        );
    }
}

// REQ-GRV-006 / F-6 — Decision: a failure means the swarm's command handler awaits inside the swarm loop, so one slow command parks gossip for every peer.
#[test]
fn req_grv_006_handle_command_has_no_inner_await() {
    let body = fn_body(&service("command_handling.rs"), "handle_command");
    assert_absent("handle_command body", &body, &[".await"]);
}

// REQ-GRV-006 / F-11 — Decision: a failure means NetworkService keeps a clone of the event Sender, so the lanes never close when the swarm exits and the node loop waits forever at shutdown.
#[test]
fn req_grv_006_f11_network_service_holds_lanes_not_an_event_sender() {
    let src = code(&service("mod.rs"));
    let at = src
        .find("pub struct NetworkService")
        .expect("struct NetworkService");
    let open = at + src[at..].find('{').expect("struct body");
    let fields = balanced(&src, open);
    assert!(
        !fields.contains("Sender<NetworkEvent>"),
        "NetworkService holds an event Sender:\n{fields}"
    );
    assert!(
        fields.contains("EventLanes"),
        "NetworkService must own the EventLanes receiver:\n{fields}"
    );
}

// REQ-GRV-007 / F-15 — Decision: a failure means an RPC tx or vote broadcast spawns one task per item again, so an RPC flood grows an unbounded task heap and an admitted tx can be left unbroadcast.
#[test]
fn req_grv_007_rpc_broadcast_wiring_never_spawns_per_item() {
    let startup = code(&node("startup.rs"));
    let mut calls = 0;
    for name in ["with_broadcast(", "with_broadcast_vote("] {
        let mut from = 0;
        while let Some(i) = startup[from..].find(name) {
            let open = from + i + name.len() - 1;
            let args = balanced(&startup, open);
            assert_absent(&format!("startup.rs {name}..)"), args, &["tokio::spawn"]);
            calls += 1;
            from = open + 1;
        }
    }
    assert!(
        calls >= 2,
        "startup.rs must wire both RPC broadcast closures"
    );
    let rb = node("rpc_broadcast.rs");
    assert_absent("rpc_broadcast.rs", &rb, &["tokio::spawn", ".await"]);
    for f in [
        "fn reserve_tx_broadcast(",
        "fn broadcast_vote_now(",
        "try_reserve",
    ] {
        assert!(code(&rb).contains(f), "rpc_broadcast.rs lacks `{f}`");
    }
}

// REQ-GRV-007 / R3 — Decision: a failure means a producer digest or announcement event spawns a task per event, so a producer-gossip flood grows an unbounded task heap; or the digest reply awaits a full command channel inside the node loop.
#[test]
fn req_grv_007_producer_gossip_handlers_never_spawn_per_event() {
    let src = node("network_events.rs");
    for name in ["on_producer_digest", "on_producers_announced"] {
        let body = fn_body(&src, name);
        assert_absent(name, &body, &["tokio::spawn"]);
    }
    let digest = fn_body(&src, "on_producer_digest");
    assert!(
        digest.contains("try_send("),
        "on_producer_digest must reply with try_send"
    );
    assert_absent("on_producer_digest", &digest, &[".send("]);
}
