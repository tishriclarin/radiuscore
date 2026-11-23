//! Example/log decoding and bounded analysis of DHCP and RADIUS Option 82 values.

pub const MAX_OPTION82_BYTES: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidHex,
    InvalidEscape,
    Truncated,
    TooLong,
    DuplicateCircuitId,
    DuplicateRemoteId,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AgentIds<'a> {
    pub circuit_id: Option<&'a [u8]>,
    pub remote_id: Option<&'a [u8]>,
}

/// Decode the bytes inside DHCP option 82, after the outer DHCP code/length.
pub fn decode_dhcp_option(value: &[u8]) -> Result<AgentIds<'_>, Error> {
    if value.len() > MAX_OPTION82_BYTES {
        return Err(Error::TooLong);
    }
    let mut ids = AgentIds::default();
    let mut remaining = value;
    while !remaining.is_empty() {
        let header = remaining.get(..2).ok_or(Error::Truncated)?;
        let length = usize::from(header[1]);
        let field = remaining.get(2..2 + length).ok_or(Error::Truncated)?;
        match header[0] {
            1 if ids.circuit_id.is_some() => return Err(Error::DuplicateCircuitId),
            2 if ids.remote_id.is_some() => return Err(Error::DuplicateRemoteId),
            1 => ids.circuit_id = Some(field),
            2 => ids.remote_id = Some(field),
            _ => {}
        }
        remaining = &remaining[2 + length..];
    }
    Ok(ids)
}

/// Parse hex diagnostic text. This is intended for fixture/CLI input.
pub fn decode_hex(value: &str) -> Result<Vec<u8>, Error> {
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let bytes = value.as_bytes();
    if bytes.len() > MAX_OPTION82_BYTES * 2 {
        return Err(Error::TooLong);
    }
    let chunks = bytes.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return Err(Error::InvalidHex);
    }
    fn nibble(value: u8) -> Result<u8, Error> {
        match value {
            b'0'..=b'9' => Ok(value - b'0'),
            b'a'..=b'f' => Ok(value - b'a' + 10),
            b'A'..=b'F' => Ok(value - b'A' + 10),
            _ => Err(Error::InvalidHex),
        }
    }
    chunks
        .map(|pair| Ok(nibble(pair[0])? * 16 + nibble(pair[1])?))
        .collect()
}

/// Decode the three-digit octal and common named escapes shown in public logs.
pub fn decode_escaped_log(value: &str) -> Result<Vec<u8>, Error> {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len().min(MAX_OPTION82_BYTES));
    let mut index = 0;
    while index < bytes.len() {
        if output.len() == MAX_OPTION82_BYTES {
            return Err(Error::TooLong);
        }
        let mut byte = bytes[index];
        index += 1;
        if byte == b'\\' {
            let escape = *bytes.get(index).ok_or(Error::InvalidEscape)?;
            index += 1;
            byte = match escape {
                b'r' => b'\r',
                b'n' => b'\n',
                b't' => b'\t',
                b'\\' | b'"' => escape,
                b'0'..=b'3' => {
                    let tail = bytes.get(index..index + 2).ok_or(Error::InvalidEscape)?;
                    if !tail.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
                        return Err(Error::InvalidEscape);
                    }
                    index += 2;
                    (escape - b'0') * 64 + (tail[0] - b'0') * 8 + (tail[1] - b'0')
                }
                _ => return Err(Error::InvalidEscape),
            };
        }
        output.push(byte);
    }
    Ok(output)
}

pub fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(value.len() * 2);
    for byte in value {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}
