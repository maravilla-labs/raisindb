//! A package install never narrows a workspace's allowed node types.
//!
//! Every install mode merges the two type lists ADD-ONLY:
//!
//! - `skip` keeps the installed workspace and adds the types the package
//!   newly lists (a package upgrade that brings a new node type makes it
//!   usable at once; before, only `allowed_node_types` was widened and a new
//!   ROOT type stayed refused).
//! - `sync` / `overwrite` take the package's definition and keep every type
//!   the installation allowed besides (a Builder-made `local:*` type, an
//!   operator's addition): dropping one would leave existing nodes of that
//!   type in a workspace that refuses them.
//!
//! A list that allows everything (`*` or empty) is never turned into a
//! restricted one.

use raisin_models::workspace::Workspace;

/// Add to `into` each allowed / root type of `from` it lacks. Returns whether
/// `into` changed.
pub(super) fn add_missing_types(into: &mut Workspace, from: &Workspace) -> bool {
    let nodes = add_missing(&mut into.allowed_node_types, &from.allowed_node_types);
    let roots = add_missing(
        &mut into.allowed_root_node_types,
        &from.allowed_root_node_types,
    );
    nodes || roots
}

fn allows_all(list: &[String]) -> bool {
    list.is_empty() || list.iter().any(|t| t == "*")
}

fn add_missing(into: &mut Vec<String>, from: &[String]) -> bool {
    if allows_all(into) {
        return false;
    }
    if allows_all(from) {
        // The other side allowed everything: keep allowing everything.
        let widened = !into.iter().any(|t| t == "*");
        into.clear();
        into.push("*".to_string());
        return widened;
    }
    let mut changed = false;
    for t in from {
        if !into.contains(t) {
            into.push(t.clone());
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::add_missing_types;
    use raisin_models::workspace::Workspace;

    fn ws(nodes: &[&str], roots: &[&str]) -> Workspace {
        let mut w = Workspace::new("app".into());
        w.allowed_node_types = nodes.iter().map(|s| s.to_string()).collect();
        w.allowed_root_node_types = roots.iter().map(|s| s.to_string()).collect();
        w
    }

    /// Skip mode: an upgrade's new type and new ROOT type become allowed.
    #[test]
    fn an_upgrade_widens_both_lists_of_the_installed_workspace() {
        let mut installed = ws(&["app:Folder", "app:Order"], &["app:Folder"]);
        let package = ws(
            &["app:Folder", "app:Order", "app:Queue"],
            &["app:Folder", "app:Queue"],
        );
        assert!(add_missing_types(&mut installed, &package));
        assert_eq!(
            installed.allowed_node_types,
            vec!["app:Folder", "app:Order", "app:Queue"]
        );
        assert_eq!(
            installed.allowed_root_node_types,
            vec!["app:Folder", "app:Queue"]
        );
        // Nothing new: no change.
        assert!(!add_missing_types(&mut installed, &package));
    }

    /// Sync / overwrite: the package's list, plus what the installation
    /// added; nothing is removed.
    #[test]
    fn a_reinstall_keeps_a_type_the_installation_added() {
        let mut package = ws(&["raisin:Asset", "raisin:Folder"], &["raisin:Folder"]);
        let live = ws(
            &["raisin:Asset", "raisin:Folder", "local:Recipe"],
            &["raisin:Folder", "local:Recipe"],
        );
        add_missing_types(&mut package, &live);
        assert_eq!(
            package.allowed_node_types,
            vec!["raisin:Asset", "raisin:Folder", "local:Recipe"]
        );
        assert_eq!(
            package.allowed_root_node_types,
            vec!["raisin:Folder", "local:Recipe"]
        );
    }

    #[test]
    fn an_unrestricted_list_stays_unrestricted() {
        // The package allows everything: the merge does not restrict it.
        let mut package = ws(&["*"], &[]);
        assert!(!add_missing_types(
            &mut package,
            &ws(&["local:X"], &["local:X"])
        ));
        assert_eq!(package.allowed_node_types, vec!["*"]);
        assert!(package.allowed_root_node_types.is_empty());

        // The installation allowed every root type: still does.
        let mut package = ws(&["a:A"], &["a:A"]);
        add_missing_types(&mut package, &ws(&["a:A"], &[]));
        assert_eq!(package.allowed_root_node_types, vec!["*"]);
    }
}
