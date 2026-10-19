// SPDX-FileCopyrightText: 2026 Mohamed Hammad <Mohamed.Hammad@SpacecraftSoftware.org>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Monitor identity from an EDID block (VESA E-EDID 1.4, base block only).
//!
//! `RandR` names outputs after connectors (`DP-1`), which change when a
//! cable moves; profiles match monitors by make, model and serial, which
//! come from here.

/// What the EDID says about the monitor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    /// Three-letter PNP manufacturer ID, e.g. `DEL`.
    pub make: Option<String>,
    /// Display product name descriptor, else the product code in hex.
    pub model: Option<String>,
    /// Serial number descriptor, else the numeric serial when non-zero.
    pub serial: Option<String>,
}

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
const BASE_LEN: usize = 128;
const DESCRIPTORS: [usize; 4] = [54, 72, 90, 108];
const TAG_SERIAL: u8 = 0xFF;
const TAG_NAME: u8 = 0xFC;

/// Parses the base EDID block. Returns an empty identity for anything that
/// is not a valid base block, since a missing identity only weakens profile
/// matching.
#[must_use]
pub fn parse(edid: &[u8]) -> Identity {
    let Some(block) = edid.get(..BASE_LEN) else {
        return Identity::default();
    };
    if block[..8] != HEADER || block.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != 0 {
        return Identity::default();
    }
    let id = u16::from_be_bytes([block[8], block[9]]);
    let letter = |shift: u16| {
        let code = (id >> shift) & 0x1F;
        u8::try_from(code)
            .ok()
            .filter(|c| (1..=26).contains(c))
            .map(|c| char::from(b'A' + c - 1))
    };
    let make: Option<String> = [letter(10), letter(5), letter(0)].into_iter().collect();
    let product = u16::from_le_bytes([block[10], block[11]]);
    let number = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);

    let mut name = None;
    let mut serial_text = None;
    for offset in DESCRIPTORS {
        let descriptor = &block[offset..offset + 18];
        // Display descriptors start with a zero pixel clock.
        if descriptor[..3] != [0, 0, 0] {
            continue;
        }
        let text = descriptor_text(&descriptor[5..]);
        match descriptor[3] {
            TAG_NAME => name = text,
            TAG_SERIAL => serial_text = text,
            _ => {}
        }
    }
    Identity {
        make,
        model: name.or_else(|| (product != 0).then(|| format!("0x{product:04X}"))),
        serial: serial_text.or_else(|| (number != 0).then(|| number.to_string())),
    }
}

/// Descriptor text is up to 13 ASCII bytes, ended by a line feed and padded
/// with spaces.
fn descriptor_text(bytes: &[u8]) -> Option<String> {
    let text: String = bytes
        .iter()
        .take_while(|b| **b != b'\n' && **b != 0)
        .filter(|b| b.is_ascii_graphic() || **b == b' ')
        .map(|b| char::from(*b))
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic base block: manufacturer `DEL`, product 0xA0B1, serial
    /// 12345, name and serial descriptors.
    fn sample() -> Vec<u8> {
        let mut block = vec![0u8; BASE_LEN];
        block[..8].copy_from_slice(&HEADER);
        // D=4, E=5, L=12, five bits each.
        let id: u16 = 0b0_00100_00101_01100;
        block[8..10].copy_from_slice(&id.to_be_bytes());
        block[10..12].copy_from_slice(&0xA0B1u16.to_le_bytes());
        block[12..16].copy_from_slice(&12_345u32.to_le_bytes());
        let mut put = |offset: usize, tag: u8, text: &[u8]| {
            block[offset + 3] = tag;
            let field = &mut block[offset + 5..offset + 18];
            field.fill(b' ');
            field[..text.len()].copy_from_slice(text);
            if text.len() < 13 {
                field[text.len()] = b'\n';
            }
        };
        put(72, TAG_NAME, b"DELL U2720Q");
        put(90, TAG_SERIAL, b"7XK2B33");
        let sum = block[..127].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        block[127] = 0u8.wrapping_sub(sum);
        block
    }

    #[test]
    fn reads_make_name_and_serial_text() {
        let identity = parse(&sample());
        assert_eq!(identity.make.as_deref(), Some("DEL"));
        assert_eq!(identity.model.as_deref(), Some("DELL U2720Q"));
        assert_eq!(identity.serial.as_deref(), Some("7XK2B33"));
    }

    #[test]
    fn falls_back_to_codes_without_descriptors() {
        let mut block = sample();
        block[75] = 0x10; // dummy descriptor instead of the name
        block[93] = 0x10;
        let sum = block[..127].iter().fold(0u8, |s, b| s.wrapping_add(*b));
        block[127] = 0u8.wrapping_sub(sum);
        let identity = parse(&block);
        assert_eq!(identity.model.as_deref(), Some("0xA0B1"));
        assert_eq!(identity.serial.as_deref(), Some("12345"));
    }

    #[test]
    fn rejects_bad_blocks() {
        assert_eq!(parse(&[]), Identity::default());
        let mut block = sample();
        block[20] ^= 1; // checksum now wrong
        assert_eq!(parse(&block), Identity::default());
    }
}
