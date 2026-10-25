//! Read NDEF records from PC/SC readers and send card selections to the launcher.

use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::Duration;

use pcsc::{Card, Context, Protocols, Scope, ShareMode};

pub(crate) const MAX_NDEF_SIZE: usize = 4096;

pub fn start() {
    // A PN532 on a serial adapter is no PC/SC reader: drive it through libnfc
    // when libnfc.conf names one. Every other reader goes through PC/SC.
    #[cfg(target_os = "linux")]
    if let Some(device) = crate::libnfc::configured_device() {
        crate::libnfc::start(device);
        return;
    }
    if let Err(error) = thread::Builder::new()
        .name("pc-gamepak-nfc".into())
        .spawn(monitor)
    {
        crate::log::line(&format!("could not start NFC reader monitor: {error}"));
    }
}

fn monitor() {
    crate::log::line("NFC reader monitor starting");
    let mut selected: HashMap<String, Vec<u8>> = HashMap::new();
    let mut launchers = Vec::new();
    loop {
        launchers
            .retain_mut(|child: &mut std::process::Child| !matches!(child.try_wait(), Ok(Some(_))));
        let context = match Context::establish(Scope::User) {
            Ok(context) => context,
            Err(error) => {
                crate::log::line(&format!("PC/SC is unavailable: {error}"));
                thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        let readers = match context.list_readers_owned() {
            Ok(readers) => readers,
            Err(error) => {
                crate::log::line(&format!("could not list PC/SC readers: {error}"));
                thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        let mut active = HashSet::new();
        for reader in readers {
            let name = reader.to_string_lossy().into_owned();
            if let Ok(card) = context.connect(&reader, ShareMode::Shared, Protocols::ANY) {
                active.insert(name.clone());
                if let Ok(message) = read_ndef(&card) {
                    if selected.get(&name) != Some(&message) {
                        selected.insert(name, message.clone());
                        crate::log::line("NFC GamePak card detected");
                        if let Some(child) = crate::launcher::open_nfc(&message) {
                            launchers.push(child);
                        }
                    }
                }
            } else {
                selected.remove(&name);
            }
        }
        selected.retain(|reader, _| active.contains(reader));
        thread::sleep(Duration::from_secs(1));
    }
}

fn read_ndef(card: &Card) -> Result<Vec<u8>, String> {
    read_type2(card).or_else(|_| read_type4(card))
}

fn read_type2(card: &Card) -> Result<Vec<u8>, String> {
    let mut memory = Vec::new();
    for page in 4u8..=u8::MAX {
        let response = exchange(card, &[0xff, 0xb0, 0x00, page, 0x04])?;
        if response.len() != 4 {
            return Err("Type 2 page read returned an unexpected length".into());
        }
        memory.extend_from_slice(&response);
        if memory.len() > MAX_NDEF_SIZE {
            return Err("NDEF data exceeds the supported size".into());
        }
        if let Some(message) = find_type2_ndef(&memory)? {
            return Ok(message);
        }
    }
    Err("no NDEF TLV found on card".into())
}

pub(crate) fn find_type2_ndef(memory: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let mut cursor = 0usize;
    while cursor < memory.len() {
        let kind = memory[cursor];
        cursor += 1;
        match kind {
            0x00 => continue,
            0xfe => return Ok(None),
            _ => {}
        }
        let Some(&first_length) = memory.get(cursor) else {
            return Ok(None);
        };
        cursor += 1;
        let length = if first_length == 0xff {
            let Some(bytes) = memory.get(cursor..cursor.saturating_add(2)) else {
                return Ok(None);
            };
            cursor += 2;
            usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))
        } else {
            usize::from(first_length)
        };
        let end = cursor
            .checked_add(length)
            .ok_or_else(|| "NDEF TLV length overflow".to_string())?;
        let Some(value) = memory.get(cursor..end) else {
            return Ok(None);
        };
        if kind == 0x03 {
            if length > MAX_NDEF_SIZE {
                return Err("NDEF data exceeds the supported size".into());
            }
            return Ok(Some(value.to_vec()));
        }
        cursor = end;
    }
    Ok(None)
}

fn read_type4(card: &Card) -> Result<Vec<u8>, String> {
    exchange(
        card,
        &[
            0x00, 0xa4, 0x04, 0x00, 0x07, 0xd2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01,
        ],
    )?;
    exchange(card, &[0x00, 0xa4, 0x00, 0x0c, 0x02, 0xe1, 0x03])?;
    let cc = exchange(card, &[0x00, 0xb0, 0x00, 0x00, 0x0f])?;
    let control = cc
        .windows(8)
        .find(|tlv| tlv[0] == 0x04 && tlv[1] == 0x06)
        .ok_or_else(|| "NDEF capability container has no file-control TLV".to_string())?;
    let file_id = [control[2], control[3]];
    let max_size = usize::from(u16::from_be_bytes([control[4], control[5]]));
    if max_size < 2 {
        return Err("NDEF file is too small".into());
    }
    exchange(
        card,
        &[0x00, 0xa4, 0x00, 0x0c, 0x02, file_id[0], file_id[1]],
    )?;
    let length = exchange(card, &[0x00, 0xb0, 0x00, 0x00, 0x02])?;
    if length.len() != 2 {
        return Err("NDEF file has no length field".into());
    }
    let size = usize::from(u16::from_be_bytes([length[0], length[1]]));
    if size == 0 || size > MAX_NDEF_SIZE || size + 2 > max_size {
        return Err("NDEF file length is outside the supported range".into());
    }
    let mut message = Vec::with_capacity(size);
    let mut offset = 2usize;
    while message.len() < size {
        let count = (size - message.len()).min(0xf0);
        let apdu = [0x00, 0xb0, (offset >> 8) as u8, offset as u8, count as u8];
        let bytes = exchange(card, &apdu)?;
        if bytes.len() != count {
            return Err("short read from NDEF file".into());
        }
        message.extend_from_slice(&bytes);
        offset += count;
    }
    Ok(message)
}

fn exchange(card: &Card, command: &[u8]) -> Result<Vec<u8>, String> {
    let mut buffer = [0u8; 260];
    let response = card
        .transmit(command, &mut buffer)
        .map_err(|error| error.to_string())?;
    if response.len() < 2 || response[response.len() - 2..] != [0x90, 0x00] {
        return Err("reader rejected NDEF command".into());
    }
    Ok(response[..response.len() - 2].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ndef_after_other_type2_tlvs() {
        let message = b"\xd1\x01\x05U\x00test";
        let mut data = vec![0x01, 0x02, 0xaa, 0xbb, 0x03, message.len() as u8];
        data.extend_from_slice(message);
        assert_eq!(find_type2_ndef(&data).unwrap(), Some(message.to_vec()));
    }

    #[test]
    fn handles_extended_tlv_length_and_incomplete_data() {
        let message = vec![0x55; 260];
        let mut data = vec![0x03, 0xff, 0x01, 0x04];
        data.extend_from_slice(&message);
        assert_eq!(find_type2_ndef(&data).unwrap(), Some(message));
        assert_eq!(find_type2_ndef(&[0x03, 0x05, 0xd1]).unwrap(), None);
    }
}
