//! The screen reader's tree of the main window: built from the same hit
//! list the window hit-tests and draws, plus each hit's description from its
//! page (`pages::describe`), so the tree cannot drift from the painted
//! controls. Pure: the UI Automation provider (`uia`) only publishes and
//! reads it.

use crate::{
    fluent::RectF,
    pages::{self, Hit, Model, Role},
};

/// One accessible element. `id` is the window's focus id (nav 1..=4,
/// page hits `hit_base + id`), the same number keyboard focus uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: u32,
    pub name: String,
    pub role: Role,
    /// Client pixels; empty when the control is scrolled out of view.
    pub rect: RectF,
    pub toggled: Option<bool>,
    pub expanded: Option<bool>,
    pub selected: bool,
}

/// A nav rail item: focus id, label, rect, and whether it is the page shown.
pub struct NavItem<'a> {
    pub id: u32,
    pub label: &'a str,
    pub rect: RectF,
    pub current: bool,
}

/// The nav rail, then the page's hits in layout order (the Tab order). A
/// hit its page cannot describe is left out rather than shown unnamed.
pub fn tree(
    nav: &[NavItem],
    page: crate::main_window::Page,
    hits: &[Hit],
    hit_base: u32,
    m: &Model,
) -> Vec<Node> {
    let mut v: Vec<Node> = nav
        .iter()
        .map(|n| Node {
            id: n.id,
            name: n.label.to_owned(),
            role: Role::Tab,
            rect: n.rect,
            toggled: None,
            expanded: None,
            selected: n.current,
        })
        .collect();
    v.extend(hits.iter().filter_map(|h| {
        let d = pages::describe(page, h, m)?;
        Some(Node {
            id: hit_base + h.id,
            name: d.name,
            role: d.role,
            rect: h.r,
            toggled: d.toggled,
            expanded: d.expanded,
            selected: d.selected,
        })
    }));
    v
}

/// The flyout's tiles as a tree: one toggle per tile, named by its label, with its on state. Built from the
/// same tiles the flyout draws, so the screen reader cannot drift from the painted tiles.
pub fn flyout_tree(tiles: &[crate::flyout_tiles::Tile]) -> Vec<Node> {
    tiles
        .iter()
        .enumerate()
        .map(|(i, t)| Node {
            id: i as u32 + 1,
            name: t.label.clone(),
            role: Role::Toggle,
            rect: crate::fluent::rectf(0.0, 0.0, 0.0, 0.0),
            toggled: Some(t.on),
            expanded: None,
            selected: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flyout_tiles::{Tile, TileKind};

    fn tile(kind: TileKind, label: &str, on: bool) -> Tile {
        Tile {
            kind,
            label: label.into(),
            on,
            enabled: true,
        }
    }

    #[test]
    fn flyout_tree_names_every_tile_and_its_state() {
        let tiles = vec![
            tile(TileKind::Camera, "Camera", true),
            tile(TileKind::Audio, "Audio", false),
        ];
        let nodes = flyout_tree(&tiles);
        assert_eq!(nodes.len(), tiles.len(), "one node per tile");
        assert_eq!(nodes[0].name, "Camera");
        assert_eq!(nodes[0].role, Role::Toggle);
        assert_eq!(nodes[0].toggled, Some(true));
        assert_eq!(nodes[1].name, "Audio");
        assert_eq!(nodes[1].toggled, Some(false));
        // Ids are unique, so focus can name a node.
        assert_ne!(nodes[0].id, nodes[1].id);
    }
    use crate::fluent::rectf;
    use crate::main_window::Page;
    use crate::pages::devices::DevicesModel;

    #[test]
    fn nav_comes_first_and_ids_are_unique_and_named() {
        let m = Model {
            devices: DevicesModel {
                devices: vec![("Pixel 8".into(), "AB".into(), false)],
            },
            ..Default::default()
        };
        let hits = pages::layout(
            Page::Devices,
            rectf(0.0, 0.0, 900.0, 600.0),
            rectf(0.0, 0.0, 900.0, 600.0),
            1.0,
            &m,
        );
        let nav: Vec<NavItem> = ["Home", "Timeline", "Devices", "Settings"]
            .iter()
            .enumerate()
            .map(|(k, l)| NavItem {
                id: k as u32 + 1,
                label: l,
                rect: rectf(0.0, k as f32 * 70.0, 80.0, k as f32 * 70.0 + 64.0),
                current: k == 2,
            })
            .collect();
        let t = tree(&nav, Page::Devices, &hits, 100, &m);
        assert_eq!(t.len(), 4 + hits.len());
        assert!(t[..4].iter().all(|n| n.role == Role::Tab));
        assert!(t[2].selected && !t[0].selected);
        let mut ids: Vec<u32> = t.iter().map(|n| n.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), t.len());
        assert!(t.iter().all(|n| !n.name.is_empty()));
        assert!(t.iter().any(|n| n.name == "Remove Pixel 8"));
    }
}
