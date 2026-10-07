//! The Pocket PC top menu bar as a data model.
//!
//! Pocket PC 2002 games build their top bar with `SHCreateMenuBar` plus
//! `InsertMenuW`/`DeleteMenu`/`CheckMenuItem` edits. PocketHLE keeps the
//! resulting structure in [`MenuBarState`] so the frontend chrome (the
//! Android toolbar, the desktop top bar) can render real buttons instead
//! of scraping the guest's pixels. Tapping a button comes back into the
//! guest as a `WM_COMMAND` with the item's command id — exactly what the
//! real control sends.
//!
//! Two sources feed the state:
//!
//! * A `SHMENUBARINFO` `nToolBarId` resource. On real hardware that is a
//!   toolbar/menubar template (TBBUTTON-like words ending in a null
//!   separator); embedded `RT_MENU` templates referenced by string table
//!   id give the labels, so games whose toolbar id doubles as a menu id
//!   resolve. Parsers live in `pocket_pe::resources`.
//! * Guest edits through `InsertMenuW`/`DeleteMenu`/`RemoveMenu` /
//!   `CheckMenuItem`/`EnableMenuItem` / `TB_INSERTBUTTON` /
//!   `TB_GETBUTTONINFO`, which mutate the bar in place.

use pocket_pe::resources::MenuItem;

/// One top-level (or nested) menu bar entry as the chrome should show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuBarItem {
    /// Command id posted as `WM_COMMAND` `wParam` low word when tapped.
    /// Zero for popups and separators.
    pub id: u16,
    /// Label text; empty for separators.
    pub label: String,
    /// Check-mark state from the template or `CheckMenuItem`.
    pub checked: bool,
    /// Grayed/disabled state from the template or `EnableMenuItem`.
    pub disabled: bool,
    /// Separator line (no id, no label).
    pub separator: bool,
    /// Nested items for a popup ("Game" → New Game / Pause / Exit).
    pub children: Vec<MenuBarItem>,
}

impl MenuBarItem {
    fn from_template(item: &MenuItem) -> Self {
        Self {
            id: item.id,
            label: item.label.clone(),
            checked: item.checked,
            disabled: item.disabled,
            separator: item.separator,
            children: item.children.iter().map(Self::from_template).collect(),
        }
    }
}

/// The active top bar. `Some` once the guest creates one.
#[derive(Debug, Clone, Default)]
pub struct MenuBarState {
    /// Top-level entries in bar order (the two fixed slots on Pocket PC).
    pub items: Vec<MenuBarItem>,
}

impl MenuBarState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the whole bar from a parsed `RT_MENU` template.
    pub fn set_from_template(&mut self, items: &[MenuItem]) {
        self.items = items.iter().map(MenuBarItem::from_template).collect();
    }

    fn find_mut(&mut self, id: u16) -> Option<&mut MenuBarItem> {
        for item in &mut self.items {
            if item.id == id {
                return Some(item);
            }
            for child in &mut item.children {
                if child.id == id {
                    return Some(child);
                }
            }
        }
        None
    }

    /// Mirror a guest `InsertMenuW`/`AppendMenu` on the bar.
    pub fn insert_item(
        &mut self,
        menu: u32,
        id: u16,
        label: String,
        flags: u32,
        popup_children: Option<Vec<MenuBarItem>>,
    ) {
        const MF_POPUP: u32 = 0x0010;
        const MF_SEPARATOR: u32 = 0x0800;
        let item = MenuBarItem {
            id: if flags & MF_POPUP != 0 { 0 } else { id },
            label: if flags & MF_SEPARATOR != 0 {
                String::new()
            } else {
                label
            },
            checked: false,
            disabled: flags & 0x0003 != 0,
            separator: flags & MF_SEPARATOR != 0,
            children: popup_children.unwrap_or_default(),
        };
        let top_level = menu == 0;
        if top_level {
            self.items.push(item);
        } else if let Some(parent) = self.find_mut(menu as u16) {
            parent.children.push(item);
        }
    }

    /// Mirror a guest `DeleteMenu`/`RemoveMenu`.
    pub fn remove_item(&mut self, menu: u32, id: u16) {
        let top_level = menu == 0;
        if top_level {
            self.items.retain(|item| item.id != id);
        } else if let Some(parent) = self.find_mut(menu as u16) {
            parent.children.retain(|child| child.id != id);
        }
    }

    /// Mirror a guest `CheckMenuItem`; returns the new state so the API
    /// round-trips like the real control.
    pub fn set_checked(&mut self, id: u16, checked: bool) -> bool {
        match self.find_mut(id) {
            Some(item) => {
                item.checked = checked;
                checked
            }
            None => checked,
        }
    }

    /// Mirror a guest `EnableMenuItem`.
    pub fn set_enabled(&mut self, id: u16, enabled: bool) {
        if let Some(item) = self.find_mut(id) {
            item.disabled = !enabled;
        }
    }

    /// Update a label in place (e.g. Pause ⇄ Resume text swaps).
    pub fn set_label(&mut self, id: u16, label: String) {
        if let Some(item) = self.find_mut(id) {
            item.label = label;
        }
    }

    /// `true` when the bar has nothing to show.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: u16, label: &str) -> MenuBarItem {
        MenuBarItem {
            id,
            label: label.into(),
            checked: false,
            disabled: false,
            separator: false,
            children: vec![],
        }
    }

    #[test]
    fn template_items_keep_structure() {
        let mut state = MenuBarState::new();
        let popup = MenuItem {
            id: 0,
            label: "Game".into(),
            checked: false,
            disabled: false,
            separator: false,
            children: vec![
                MenuItem {
                    id: 0x9C41,
                    label: "Sound".into(),
                    checked: false,
                    disabled: false,
                    separator: false,
                    children: vec![],
                },
                MenuItem {
                    id: 0x9C45,
                    label: "Exit".into(),
                    checked: false,
                    disabled: false,
                    separator: false,
                    children: vec![],
                },
            ],
        };
        let pause = MenuItem {
            id: 0x9C46,
            label: "Pause".into(),
            checked: false,
            disabled: false,
            separator: false,
            children: vec![],
        };
        state.set_from_template(&[popup, pause]);
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].children.len(), 2);
        assert!(state.set_checked(0x9C46, true));
        assert!(state.items[1].checked);
        state.remove_item(0, 0x9C46);
        assert_eq!(state.items.len(), 1);
    }

    #[test]
    fn leaf_helpers_and_empty_state() {
        let state = MenuBarState::new();
        assert!(state.is_empty());
        let item = leaf(7, "Go");
        assert_eq!(item.id, 7);
        assert!(!item.separator);
    }
}
