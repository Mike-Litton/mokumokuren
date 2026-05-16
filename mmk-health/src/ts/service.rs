//! Pattern B — service / interface declaration pairs.
//!
//! Detection signal: the file declares an `interface IFoo { ... }`
//! AND either calls `registerSingleton(IFoo, ...)` or uses
//! `IFoo = createDecorator<IFoo>(...)`. When both fire the file is
//! a service-decl module, and the detector surfaces the top peer
//! files that import `IFoo`.
//!
//! Interface-name extraction reads the source in a single
//! tree-sitter pass; the cross-file consumer scan is a grep-style
//! substring match against `IFoo` in the peer paths. Good enough
//! for the `interface IFoo + registerSingleton(IFoo, FooImpl)`
//! convention; an import-statement parse would tighten precision
//! once eval data motivates the upgrade.

use crate::ts::parse_for;
use crate::{HealthFinding, HealthPattern};
use std::path::{Path, PathBuf};
use tree_sitter::Node;

/// Cap on consumer files surfaced per finding. Three keeps the
/// signal precedent-shaped (here are real importers) without
/// turning into a workbench audit.
const MAX_CONSUMERS: usize = 3;

/// Hard cap on peer files inspected before giving up. Without this,
/// a service decl whose interface name doesn't appear in the first
/// few hundred peers would scan the entire workbench (large monorepos
/// can have thousands of .ts files), each via an `fs::read_to_string`.
/// Capping keeps the worst case bounded; if the consumer is past
/// index MAX_PEERS_SCANNED it's silently missed.
const MAX_PEERS_SCANNED: usize = 500;

#[must_use]
pub fn detect(subject: &Path, body: &str, peer_paths: &[PathBuf]) -> Vec<HealthFinding> {
    let Some(tree) = parse_for(subject, body) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let interfaces = collect_interface_names(root, body);
    if interfaces.is_empty() {
        return Vec::new();
    }
    let registers = body.contains("registerSingleton(") || body.contains("createDecorator");
    if !registers {
        return Vec::new();
    }
    // For each declared interface that's wired via registerSingleton
    // / createDecorator, surface up to MAX_CONSUMERS peer files that
    // import the name. We aggregate across interfaces (a file
    // typically owns one) and dedupe.
    let mut consumers: Vec<PathBuf> = Vec::new();
    let mut scanned = 0_usize;
    'outer: for iface in &interfaces {
        for peer in peer_paths {
            if peer.as_path() == subject || consumers.contains(peer) {
                continue;
            }
            if !is_typescript_path(peer) {
                continue;
            }
            scanned += 1;
            if scanned > MAX_PEERS_SCANNED {
                break 'outer;
            }
            // Substring check is lightweight: we accept any mention
            // of the interface name as a consumer signal. False
            // positives cost an extra read for the agent; false
            // negatives let real consumers slip past.
            if let Ok(peer_body) = std::fs::read_to_string(peer) {
                if peer_body.contains(iface) {
                    consumers.push(peer.clone());
                    if consumers.len() >= MAX_CONSUMERS {
                        break 'outer;
                    }
                }
            }
        }
    }
    if consumers.is_empty() {
        return Vec::new();
    }
    consumers.sort();
    vec![HealthFinding {
        pattern: HealthPattern::Service,
        subject: subject.to_path_buf(),
        related: consumers,
        detail: None,
    }]
}

fn is_typescript_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "ts" | "tsx" | "js" | "jsx"))
}

/// Walk the AST collecting `interface IFoo` names. We recurse
/// through the tree so interfaces nested inside a `namespace` or a
/// module block also count — most service interfaces in practice
/// sit at top level, but the recursive walk is the correct
/// over-approximation.
fn collect_interface_names(node: Node<'_>, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    walk_for_interfaces(node, source, &mut names);
    names
}

fn walk_for_interfaces(node: Node<'_>, source: &str, out: &mut Vec<String>) {
    if node.kind() == "interface_declaration" {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                if name.starts_with('I') && name.chars().nth(1).is_some_and(char::is_uppercase) {
                    out.push(name.to_owned());
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_interfaces(child, source, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_iprefixed_interface_name() {
        let body = "interface IFoo { bar(): void; }\nregisterSingleton(IFoo, FooImpl);";
        let tree = parse_for(Path::new("a.ts"), body).expect("parse");
        let names = collect_interface_names(tree.root_node(), body);
        assert_eq!(names, vec!["IFoo".to_string()]);
    }

    #[test]
    fn skips_non_iprefixed_interface_name() {
        // `Foo` (no leading I) doesn't match the I-prefixed
        // service-interface convention; the detector ignores it.
        let body = "interface Foo { bar(): void; }";
        let tree = parse_for(Path::new("a.ts"), body).expect("parse");
        let names = collect_interface_names(tree.root_node(), body);
        assert!(names.is_empty(), "got {names:?}");
    }
}
