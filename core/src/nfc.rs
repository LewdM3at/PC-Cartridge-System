//! NFC transport: decode the NDEF record that carries a GamePak ID — a
//! `gamepak://gp_…` URI, or a Text record holding just the ID.
//!
//! NFC is only one trigger. All it contributes is the stable `gp_` identifier;
//! resolving the ID to an action is [`crate::gamepak`]'s job.

use crate::gamepak::GamePakId;

const NDEF_HEX_LIMIT: usize = 8192;

pub fn id_from_ndef_hex(encoded: &str) -> Result<GamePakId, String> {
    if encoded.is_empty() || encoded.len() > NDEF_HEX_LIMIT || !encoded.len().is_multiple_of(2) {
        return Err(
            "NDEF transport must be non-empty, even-length hex (max 8192 characters)".into(),
        );
    }
    let mut message = Vec::with_capacity(encoded.len() / 2);
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    for pair in pairs {
        let high =
            hex_value(pair[0]).ok_or_else(|| "NDEF transport is not hexadecimal".to_string())?;
        let low =
            hex_value(pair[1]).ok_or_else(|| "NDEF transport is not hexadecimal".to_string())?;
        message.push((high << 4) | low);
    }
    id_from_ndef(&message)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub fn id_from_ndef(message: &[u8]) -> Result<GamePakId, String> {
    let mut cursor = 0usize;
    let mut first = true;
    while cursor < message.len() {
        let header = take_byte(message, &mut cursor)?;
        let begin = header & 0x80 != 0;
        let end = header & 0x40 != 0;
        let chunked = header & 0x20 != 0;
        let short = header & 0x10 != 0;
        let has_id = header & 0x08 != 0;
        if begin != first || chunked {
            return Err(
                "NDEF record boundaries are invalid or chunked records are unsupported".into(),
            );
        }
        first = false;
        let type_len = take_byte(message, &mut cursor)? as usize;
        let payload_len = if short {
            take_byte(message, &mut cursor)? as usize
        } else {
            take_u32(message, &mut cursor)? as usize
        };
        let id_len = if has_id {
            take_byte(message, &mut cursor)? as usize
        } else {
            0
        };
        let record_type = take_bytes(message, &mut cursor, type_len)?;
        take_bytes(message, &mut cursor, id_len)?;
        let payload = take_bytes(message, &mut cursor, payload_len)?;
        if header & 0x07 == 0x01 && record_type == b"U" {
            if let Some((&prefix, body)) = payload.split_first() {
                if prefix == 0 {
                    let uri = std::str::from_utf8(body)
                        .map_err(|_| "NDEF URI is not valid UTF-8".to_string())?;
                    let gamepak_id = if let Some(id) = uri.strip_prefix("gamepak://") {
                        Some(id)
                    } else if uri
                        .get(..10)
                        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("gamepak://"))
                    {
                        Some(&uri[10..])
                    } else {
                        None
                    };
                    if let Some(id) = gamepak_id {
                        if id.contains(['/', '?', '#']) {
                            return Err("GamePak URI must contain only a GamePak ID".into());
                        }
                        if !end || cursor != message.len() {
                            return Err("GamePak URI must be the only NDEF record".into());
                        }
                        return GamePakId::parse(id);
                    }
                }
            }
        }
        // A Text record holding just the ID, as a phone's NFC app writes
        // "gp_stardew": the card carries the ID the way a barcode does.
        if header & 0x07 == 0x01 && record_type == b"T" {
            if let Some((&status, rest)) = payload.split_first() {
                let language_len = usize::from(status & 0x3f);
                let text = rest
                    .get(language_len..)
                    .filter(|_| status & 0x80 == 0) // UTF-8, not UTF-16
                    .and_then(|text| std::str::from_utf8(text).ok())
                    .map(str::trim);
                if let Some(id) = text.filter(|text| text.starts_with("gp_")) {
                    if !end || cursor != message.len() {
                        return Err("GamePak ID must be the only NDEF record".into());
                    }
                    return GamePakId::parse(id);
                }
            }
        }
        if end {
            if cursor != message.len() {
                return Err("NDEF message has trailing bytes".into());
            }
            return Err("NDEF message has no GamePak URI record".into());
        }
    }
    Err("NDEF message is incomplete".into())
}

fn take_byte(message: &[u8], cursor: &mut usize) -> Result<u8, String> {
    let byte = *message
        .get(*cursor)
        .ok_or_else(|| "NDEF record is truncated".to_string())?;
    *cursor += 1;
    Ok(byte)
}

fn take_u32(message: &[u8], cursor: &mut usize) -> Result<u32, String> {
    let bytes = take_bytes(message, cursor, 4)?;
    Ok(u32::from_be_bytes(bytes.try_into().expect("four bytes")))
}

fn take_bytes<'a>(
    message: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> Result<&'a [u8], String> {
    let end = cursor
        .checked_add(length)
        .ok_or_else(|| "NDEF record length overflow".to_string())?;
    let bytes = message
        .get(*cursor..end)
        .ok_or_else(|| "NDEF record is truncated".to_string())?;
    *cursor = end;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri_ndef(uri: &[u8]) -> Vec<u8> {
        let mut message = vec![0xd1, 0x01, (uri.len() + 1) as u8, b'U', 0];
        message.extend_from_slice(uri);
        message
    }

    #[test]
    fn parses_gamepak_uri_from_ndef() {
        let id = id_from_ndef(&uri_ndef(b"gamepak://gp_stardew-valley")).unwrap();
        assert_eq!(id.as_str(), "gp_stardew-valley");
    }

    #[test]
    fn parses_a_gamepak_id_from_a_text_record() {
        // Text record, UTF-8, language "en", as a phone writes "gp_stardew"
        let mut message = vec![0xd1, 0x01, 13, b'T', 0x02, b'e', b'n'];
        message.extend_from_slice(b"gp_stardew");
        assert_eq!(id_from_ndef(&message).unwrap().as_str(), "gp_stardew");

        let mut other = vec![0xd1, 0x01, 8, b'T', 0x02, b'e', b'n'];
        other.extend_from_slice(b"hello");
        assert!(id_from_ndef(&other).is_err());
    }

    #[test]
    fn rejects_invalid_identifiers_and_uris() {
        assert!(GamePakId::parse("gp_").is_err());
        assert!(GamePakId::parse("gp_Upper").is_err());
        assert!(id_from_ndef(&uri_ndef(b"gamepak://gp_demo/path")).is_err());
    }

    #[test]
    fn decodes_hex_transport() {
        let message = uri_ndef(b"gamepak://gp_demo");
        let encoded = message
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(id_from_ndef_hex(&encoded).unwrap().as_str(), "gp_demo");
        assert!(id_from_ndef_hex("xyz").is_err());
    }
}
