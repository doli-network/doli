#!/usr/bin/env python3
"""blast.py — deterministic, zero-token blast-radius query over a graphify graph.json.

Thin-slice OMEGA integration: answers "what depends on X?" by reading the local
knowledge graph instead of grepping the codebase. No LLM call — the graph is parsed
locally and only a compact dependent list is emitted for the agent to act on.

Graph format: graphify graphify-out/graph.json (NetworkX node-link).
  nodes: {id, label, norm_label, source_file, source_location, community, ...}
  links: {source, target, relation, confidence, confidence_score, source_location}
  The graph is undirected, but `relation` encodes direction:
    for calls/references/imports_from/uses/method/implements/inherits,
    `source` is the DEPENDENT and `target` is the DEPENDENCY.

Usage:
  blast.py GRAPH.json QUERY [--hops N] [--include-inferred] [--json]
    QUERY  symbol name (label) or a source_file path fragment.
"""
import argparse, json, sys, collections

# relations where source depends on target ("source uses target")
DEPEND_RELATIONS = {"calls", "references", "imports_from", "uses",
                    "method", "implements", "inherits"}
# structural relations (file/module -> the symbols it owns)
STRUCT_RELATIONS = {"contains", "defines"}


def load(graph_path):
    with open(graph_path) as f:
        d = json.load(f)
    nodes = {n["id"]: n for n in d["nodes"]}
    return nodes, d["links"]


def resolve(query, nodes):
    """Return the set of node ids the query refers to.
    Match priority: exact label/norm_label -> source_file fragment -> label substring.
    Symbols carry a '()' suffix in labels, so match the bare and parenthesized forms."""
    q = query.lower()
    forms = {q, q.rstrip("()"), q.rstrip("()") + "()"}
    exact = {nid for nid, n in nodes.items()
             if str(n.get("label", "")).lower() in forms
             or str(n.get("norm_label", "")).lower() in forms}
    if exact:
        return exact, "exact-label"
    byfile = {nid for nid, n in nodes.items()
              if q in str(n.get("source_file", "")).lower()}
    if byfile:
        return byfile, "source-file"
    sub = {nid for nid, n in nodes.items()
           if q in str(n.get("label", "")).lower()}
    return sub, "label-substring"


def dependents(seed_ids, links, extracted_only, hops):
    """BFS over reverse dependency edges: who depends on the seed set, up to `hops`.
    Blast radius defaults to INCLUDING inferred edges — graphify marks most cross-function
    `calls` as INFERRED, so EXTRACTED-only would silently drop real dependents (false
    negatives). Each dependent keeps its confidence tag so the caller can weight/prune."""
    incoming = collections.defaultdict(list)   # target -> list of (source, edge)
    for e in links:
        if e.get("relation") not in DEPEND_RELATIONS:
            continue
        if extracted_only and e.get("confidence") != "EXTRACTED":
            continue
        incoming[e["target"]].append((e["source"], e))
    found = {}          # dependent_id -> (edge, depth)
    frontier = set(seed_ids)
    seen = set(seed_ids)
    for depth in range(1, hops + 1):
        nxt = set()
        for tid in frontier:
            for src, e in incoming.get(tid, []):
                if src not in seen:
                    found.setdefault(src, (e, depth))
                    nxt.add(src)
                    seen.add(src)
        frontier = nxt
        if not frontier:
            break
    return found


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("graph")
    ap.add_argument("query")
    ap.add_argument("--hops", type=int, default=1)
    ap.add_argument("--extracted-only", action="store_true",
                    help="restrict to EXTRACTED edges (default includes INFERRED for completeness)")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    nodes, links = load(a.graph)
    seed, how = resolve(a.query, nodes)
    if not seed:
        print(f"No node matched '{a.query}'.", file=sys.stderr)
        sys.exit(1)
    deps = dependents(seed, links, a.extracted_only, a.hops)

    rows = []
    for nid, (e, depth) in deps.items():
        n = nodes.get(nid, {})
        rows.append({
            "label": n.get("label", nid),
            "where": f'{n.get("source_file","?")}:{n.get("source_location","?")}',
            "relation": e.get("relation"),
            "confidence": e.get("confidence"),
            "depth": depth,
        })
    rows.sort(key=lambda r: (r["depth"], r["where"]))

    if a.json:
        print(json.dumps({"query": a.query, "matched_via": how,
                          "seed_nodes": len(seed), "dependents": rows}, indent=2))
        return
    print(f"# blast radius of '{a.query}'  (matched {len(seed)} node(s) via {how}, hops={a.hops})")
    print(f"# {len(rows)} dependent(s)\n")
    byfile = collections.defaultdict(list)
    for r in rows:
        byfile[r["where"].split(":")[0]].append(r)
    for f in sorted(byfile):
        print(f"{f}")
        for r in byfile[f]:
            print(f"  {r['where'].split(':',1)[1]:>6}  {r['relation']:<12} {r['label']}")


if __name__ == "__main__":
    main()
