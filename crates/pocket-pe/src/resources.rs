//! PE resource directory parser.
//!
//! Implements just enough of the `.rsrc` directory format to flatten
//! it into a list of `(type, id_or_name) → (data_rva, size)` entries.
//! That is all PocketHLE needs for `FindResourceW`/`LoadResource`/
//! `LockResource` to return real bytes from the loaded image.
//!
//! Reference: <https://learn.microsoft.com/en-us/windows/win32/debug/pe-format#the-rsrc-section>

use byteorder::{ByteOrder, LittleEndian};
use goblin::pe::PE;

use crate::LoadError;

/// Either a named or integer-keyed resource.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceKey {
    Id(u32),
    Name(String),
}

/// One leaf node from the resource directory.
#[derive(Debug, Clone)]
pub struct ResourceEntry {
    pub ty: ResourceKey,
    pub name: ResourceKey,
    /// Resource virtual address relative to the image base.
    pub data_rva: u32,
    pub size: u32,
    pub codepage: u32,
}

/// Walk the resource directory and return one [`ResourceEntry`] per
/// leaf. Returns an empty vector if the image has no `.rsrc` section.
pub fn collect_resources(bytes: &[u8], pe: &PE) -> Result<Vec<ResourceEntry>, LoadError> {
    let mut out = Vec::new();
    let oh = match pe.header.optional_header {
        Some(h) => h,
        None => return Ok(out),
    };
    let dir = match oh.data_directories.get_resource_table() {
        Some(d) if d.size > 0 => d,
        _ => return Ok(out),
    };

    // Resolve the section that contains the resource directory so we
    // can convert sub-offsets to file offsets.
    let rsrc_section = pe.sections.iter().find(|s| {
        let start = s.virtual_address;
        let end = start.saturating_add(s.virtual_size.max(s.size_of_raw_data));
        dir.virtual_address >= start && dir.virtual_address < end
    });
    let rsrc_section = match rsrc_section {
        Some(s) => s,
        None => return Ok(out),
    };

    let section_va = rsrc_section.virtual_address;
    let section_off = rsrc_section.pointer_to_raw_data as usize;
    let section_size = rsrc_section.size_of_raw_data.max(rsrc_section.virtual_size) as usize;

    let read = |off: usize, n: usize| -> Option<&[u8]> {
        let abs = section_off + off;
        if abs + n > bytes.len() || off + n > section_size {
            None
        } else {
            Some(&bytes[abs..abs + n])
        }
    };

    let read_string = |string_off: u32| -> Option<String> {
        // IMAGE_RESOURCE_DIR_STRING_U: u16 length, then UTF-16LE chars.
        let off = string_off as usize;
        let lh = read(off, 2)?;
        let len = LittleEndian::read_u16(lh) as usize;
        let raw = read(off + 2, len * 2)?;
        let mut us = Vec::with_capacity(len);
        for c in raw.as_chunks::<2>().0 {
            us.push(LittleEndian::read_u16(c));
        }
        Some(String::from_utf16_lossy(&us))
    };

    fn parse_directory<F: FnMut(u32, bool, u32)>(
        section_off: usize,
        section_size: usize,
        bytes: &[u8],
        dir_off: u32,
        cb: &mut F,
    ) -> Option<()> {
        let dir_abs = section_off + dir_off as usize;
        if dir_abs + 16 > bytes.len() || dir_off as usize + 16 > section_size {
            return None;
        }
        let header = &bytes[dir_abs..dir_abs + 16];
        let named = LittleEndian::read_u16(&header[12..14]) as usize;
        let id = LittleEndian::read_u16(&header[14..16]) as usize;
        let total = named + id;
        let entries_abs = dir_abs + 16;
        for i in 0..total {
            let entry_abs = entries_abs + i * 8;
            if entry_abs + 8 > bytes.len() {
                break;
            }
            let entry = &bytes[entry_abs..entry_abs + 8];
            let name = LittleEndian::read_u32(&entry[0..4]);
            let off = LittleEndian::read_u32(&entry[4..8]);
            let is_dir = off & 0x8000_0000 != 0;
            let off = off & 0x7fff_ffff;
            cb(name, is_dir, off);
        }
        Some(())
    }

    let key_from = |raw: u32| -> Option<ResourceKey> {
        if raw & 0x8000_0000 != 0 {
            let off = raw & 0x7fff_ffff;
            Some(ResourceKey::Name(read_string(off).unwrap_or_default()))
        } else {
            Some(ResourceKey::Id(raw))
        }
    };

    // Three-level walk: type -> name -> language -> data.
    let root = dir.virtual_address - section_va;
    let mut type_entries: Vec<(u32, u32)> = Vec::new();
    parse_directory(
        section_off,
        section_size,
        bytes,
        root,
        &mut |name, is_dir, off| {
            if is_dir {
                type_entries.push((name, off));
            }
        },
    );
    for (type_raw, type_dir_off) in type_entries {
        let ty = match key_from(type_raw) {
            Some(k) => k,
            None => continue,
        };
        let mut name_entries: Vec<(u32, u32)> = Vec::new();
        parse_directory(
            section_off,
            section_size,
            bytes,
            type_dir_off,
            &mut |name, is_dir, off| {
                if is_dir {
                    name_entries.push((name, off));
                }
            },
        );
        for (name_raw, name_dir_off) in name_entries {
            let name = match key_from(name_raw) {
                Some(k) => k,
                None => continue,
            };
            // Pick the first language in this name's directory — that
            // is what `LoadResource` does when LANG_NEUTRAL is asked.
            let mut lang_entries: Vec<u32> = Vec::new();
            parse_directory(
                section_off,
                section_size,
                bytes,
                name_dir_off,
                &mut |_name, is_dir, off| {
                    if !is_dir {
                        lang_entries.push(off);
                    }
                },
            );
            if let Some(data_entry_off) = lang_entries.first() {
                let abs = section_off + (*data_entry_off as usize);
                if abs + 16 > bytes.len() {
                    continue;
                }
                let data_rva = LittleEndian::read_u32(&bytes[abs..abs + 4]);
                let size = LittleEndian::read_u32(&bytes[abs + 4..abs + 8]);
                let codepage = LittleEndian::read_u32(&bytes[abs + 8..abs + 12]);
                out.push(ResourceEntry {
                    ty: ty.clone(),
                    name,
                    data_rva,
                    size,
                    codepage,
                });
            }
        }
    }
    Ok(out)
}

/// One entry of a Windows menu, parsed from a `RT_MENU` template.
///
/// This is the model the host chrome (the Android top bar) renders:
/// a menu bar is a list of buttons, and each button is either a
/// direct command or a popup carrying nested [`MenuItem`] children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// Command id the item posts (`WM_COMMAND` `wParam` low word).
    /// Zero for popups and separators.
    pub id: u16,
    /// Label text. Empty for separators.
    pub label: String,
    /// `MF_CHECKED` state from the template (guest updates follow
    /// through `CheckMenuItem`).
    pub checked: bool,
    /// `MF_GRAYED` / `MF_DISABLED` state from the template.
    pub disabled: bool,
    /// `true` for a separator line (`MF_SEPARATOR`, id 0, no label).
    pub separator: bool,
    /// Nested items for an `MF_POPUP` entry; empty for plain commands.
    pub children: Vec<MenuItem>,
}

impl MenuItem {
    pub fn leaf(id: u16, label: String, option: u16) -> Self {
        Self {
            id,
            label,
            checked: option & 0x0008 != 0,
            disabled: option & 0x0003 != 0,
            separator: option & 0x0800 != 0,
            children: Vec::new(),
        }
    }
}

const MF_END: u16 = 0x0080;
const MF_POPUP: u16 = 0x0010;

#[allow(dead_code)]
const MF_GRAYED: u16 = 0x0001;
#[allow(dead_code)]
const MF_DISABLED: u16 = 0x0002;
#[allow(dead_code)]
const MF_CHECKED: u16 = 0x0008;
#[allow(dead_code)]
const MF_SEPARATOR: u16 = 0x0800;

fn read_u16(data: &[u8], off: usize) -> Option<u16> {
    let b = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

/// Read a NUL-terminated UTF-16 string starting at `off`. Returns the
/// string and the offset just past the terminator.
fn read_utf16z(data: &[u8], off: usize) -> Option<(String, usize)> {
    let mut units = Vec::new();
    let mut cur = off;
    loop {
        let unit = read_u16(data, cur)?;
        cur += 2;
        if unit == 0 {
            break;
        }
        units.push(unit);
    }
    Some((String::from_utf16_lossy(&units), cur))
}

/// Parse a `RT_MENU` resource (`MENUTEMPLATE`, Unicode — CE menus are
/// always wide). Returns the top-level items with nested popups.
///
/// Layout: a 4-byte header (`wVersion`, `cbHeaderOffset` — both zero
/// for every CE template in practice) followed by a flat stream of
/// items. Each item is `mtOption` (flags), then for non-popups
/// `mtID`, then a NUL-terminated wide string. `MF_END` marks the last
/// item at its nesting level, so popups are parsed recursively.
/// Separators are items with `MF_SEPARATOR` and no id/string, which
/// the RC compiler emits as option `0xFFFF` followed by nothing.
pub fn parse_menu_template(data: &[u8]) -> Option<Vec<MenuItem>> {
    if data.len() < 4 {
        return None;
    }
    let header_offset = read_u16(data, 2)? as usize;
    let mut pos = 4 + header_offset;
    parse_menu_items(data, &mut pos)
}

fn parse_menu_items(data: &[u8], pos: &mut usize) -> Option<Vec<MenuItem>> {
    let mut items = Vec::new();
    loop {
        if *pos + 2 > data.len() {
            return if items.is_empty() { None } else { Some(items) };
        }
        let option = read_u16(data, *pos)?;
        *pos += 2;
        let is_popup = option & MF_POPUP != 0;
        let (id, label) = if is_popup {
            // A popup carries no id word: the submenu items follow
            // the label directly.
            let (s, next) = read_utf16z(data, *pos)?;
            *pos = next;
            (0, s)
        } else {
            // Kevtris compiles separators as a bare id word of zero
            // followed by an empty string slot (`00 00 00 00 00 00`),
            // so the string is always present for non-popup items.
            let id = read_u16(data, *pos)?;
            *pos += 2;
            let (text, next) = read_utf16z(data, *pos)?;
            *pos = next;
            (id, text)
        };
        let mut item = MenuItem::leaf(id, label, option);
        if id == 0 && !is_popup {
            // Zero id = separator, whether or not the compiler
            // flagged it (some emit option 0, not MF_SEPARATOR).
            item.separator = true;
        }
        if is_popup {
            item.children = parse_menu_items(data, pos)?;
        }
        items.push(item);
        if option & MF_END != 0 {
            return Some(items);
        }
    }
}

/// The parts of a Pocket PC menu bar template the host chrome needs:
/// which `RT_MENU` resource holds the items, and the command ids of
/// the soft key buttons in left-to-right order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuBarTemplate {
    /// `RT_MENU` id referenced by the template. Some generators store
    /// the id minus one here, so callers should fall back to `id + 1`.
    pub menu_id: u32,
    /// Per-button command ids, left soft key first.
    pub button_commands: Vec<u16>,
}

/// Parse a Pocket PC menu bar `RT_RCDATA` template — the resource
/// `SHMENUBARINFO::nToolBarId` names (`SHCreateMenuBar`).
///
/// Layout as observed in PPC2000–WM5 binaries (`Kevtris`, Solitaire): a
/// two-word header (menu resource id, button count) followed by
/// `count` 14-byte button records of six words plus a `0xFFFF`
/// terminator:
///
/// ```text
/// WORD 0xFFFE      marks a soft key button
/// WORD idCommand   command id posted for the button
/// WORD iImage      image index into the strip bitmap
/// WORD wWidth      button width in pixels
/// WORD ?           generator-specific
/// WORD ?           generator-specific
/// WORD 0xFFFF      record terminator
/// ```
///
/// The item text never lives here — the labels come from the
/// `RT_MENU` resource named by the header, whose top-level items pair
/// with the buttons in order. Parsers therefore only need the menu id
/// and the button command ids.
pub fn parse_menubar_template(data: &[u8]) -> Option<MenuBarTemplate> {
    let menu_id = read_u16(data, 0)? as u32;
    let count = read_u16(data, 2)? as usize;
    if count == 0 || count > 2 || menu_id == 0 {
        return None;
    }
    if data.len() < 4 + count * 14 {
        return None;
    }
    let mut pos = 4;
    let mut button_commands = Vec::with_capacity(count);
    for _ in 0..count {
        // 0xFFFE marks a soft key button. Generators disagree on the
        // trailing record words (Kevtris ends its first record with
        // `00 00 00 00`, Solitaire-era templates with `00 00 ff ff`),
        // so only the marker and the command id are treated as data;
        // the rest of the 14-byte record is skipped.
        if read_u16(data, pos)? != 0xFFFE {
            return None;
        }
        let command = read_u16(data, pos + 2)?;
        button_commands.push(command);
        pos += 14;
    }
    Some(MenuBarTemplate {
        menu_id,
        button_commands,
    })
}

#[cfg(test)]
mod menu_tests {
    use super::*;

    #[test]
    fn kevtris_menu_template_parses_game_popup_and_pause_button() {
        // RT_MENU 102 of the Kevtris 240x320 resource DLL, byte for
        // byte: a "Game" popup with six entries, then a top-level
        // "Pause" command. The on-device soft key bar pairs these
        // with the two buttons of menubar RCDATA 102.
        let data: Vec<u8> = {
            use std::vec::Vec;
            let wide = |s: &str| -> Vec<u8> {
                s.encode_utf16()
                    .chain(std::iter::once(0))
                    .flat_map(|u| u.to_le_bytes())
                    .collect()
            };
            let mut d = Vec::new();
            d.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // header
                                                            // "Game" popup
            d.extend_from_slice(&MF_POPUP.to_le_bytes());
            d.extend_from_slice(&wide("Game"));
            //   Update Online Scores (grayed, disabled)
            d.extend_from_slice(&(MF_GRAYED | MF_DISABLED).to_le_bytes());
            d.extend_from_slice(&0x9CA6u16.to_le_bytes());
            d.extend_from_slice(&wide("Update Online Scores"));
            //   separator — Kevtris's compiler emits option 0,
            //   id 0 and an empty string slot.
            d.extend_from_slice(&0u16.to_le_bytes());
            d.extend_from_slice(&0u16.to_le_bytes());
            d.extend_from_slice(&wide(""));
            //   Sound (checked)
            d.extend_from_slice(&MF_CHECKED.to_le_bytes());
            d.extend_from_slice(&0x9C41u16.to_le_bytes());
            d.extend_from_slice(&wide("Sound"));
            //   Assign Buttons...
            d.extend_from_slice(&0u16.to_le_bytes());
            d.extend_from_slice(&0x9C51u16.to_le_bytes());
            d.extend_from_slice(&wide("Assign Buttons..."));
            //   Exit (last item of the popup: MF_END)
            d.extend_from_slice(&MF_END.to_le_bytes());
            d.extend_from_slice(&0x9C45u16.to_le_bytes());
            d.extend_from_slice(&wide("Exit"));
            // "Pause" direct command (last top-level item: MF_END)
            d.extend_from_slice(&MF_END.to_le_bytes());
            d.extend_from_slice(&0x9C46u16.to_le_bytes());
            d.extend_from_slice(&wide("Pause"));
            d
        };

        let items = parse_menu_template(&data).expect("template must parse");
        assert_eq!(items.len(), 2);

        let game = &items[0];
        assert_eq!(game.label, "Game");
        assert_eq!(game.children.len(), 5);
        assert_eq!(game.children[0].label, "Update Online Scores");
        assert!(game.children[0].disabled);
        assert!(game.children[1].separator);
        assert!(game.children[2].checked);
        assert_eq!(game.children[2].id, 0x9C41);
        assert_eq!(game.children[4].id, 0x9C45);
        assert_eq!(game.children[4].label, "Exit");

        let pause = &items[1];
        assert_eq!(pause.label, "Pause");
        assert_eq!(pause.id, 0x9C46);
        assert!(pause.children.is_empty());
    }

    #[test]
    fn kevtris_menubar_template_names_menu_and_two_buttons() {
        // RT_RCDATA 102 of the Kevtris resource DLL: menu 102, two
        // buttons. The first button record ends `00 00 00 00` —
        // generators disagree on the trailing words, which the parser
        // must tolerate.
        let data: Vec<u8> = [
            0x66, 0x00, // menu id 102
            0x02, 0x00, // 2 buttons
            0xFE, 0xFF, 0x6F, 0x9C, 0x04, 0x00, 0x18, 0x00, 0x68, 0x00, 0x00, 0x00, 0x00, 0x00,
            0xFE, 0xFF, 0x46, 0x9C, 0x0C, 0x00, 0x10, 0x00, 0x4F, 0x9C, 0x00, 0x00, 0xFF, 0xFF,
        ]
        .to_vec();

        let bar = parse_menubar_template(&data).expect("menubar template must parse");
        assert_eq!(bar.menu_id, 102);
        assert_eq!(bar.button_commands, vec![0x9C6F, 0x9C46]);
    }

    #[test]
    fn solitaire_shaped_menubar_template_terminates_with_ffff() {
        // RT_RCDATA 122 shape: Done / Cancel, every record ending
        // `00 00 ff ff`.
        let data: Vec<u8> = [
            0x7A, 0x00, 0x02, 0x00, 0xFE, 0xFF, 0x7A, 0x9C, 0x04, 0x00, 0x10, 0x00, 0x7F, 0x9C,
            0x00, 0x00, 0xFF, 0xFF, 0xFE, 0xFF, 0x7B, 0x9C, 0x04, 0x00, 0x10, 0x00, 0x6C, 0x9C,
            0x00, 0x00, 0xFF, 0xFF,
        ]
        .to_vec();

        let bar = parse_menubar_template(&data).expect("menubar template must parse");
        assert_eq!(bar.button_commands, vec![0x9C7A, 0x9C7B]);
    }

    #[test]
    fn garbage_templates_are_rejected() {
        assert!(parse_menubar_template(&[0x66, 0x00, 0x00, 0x00]).is_none());
        assert!(parse_menu_template(&[0x00, 0x00, 0x00]).is_none());
    }
}
