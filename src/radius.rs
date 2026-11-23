//! Legacy RADIUS framing and selected attribute decode. The caller must validate
//! the transport and packet authenticator before using attributes for access.

use crate::rewrite::{self, Received};

pub(crate) const HEADER_BYTES: usize = 20;
pub(crate) const MAX_PACKET_BYTES: usize = 4096;
const VENDOR_SPECIFIC: u8 = 26;
const CALLING_STATION_ID: u8 = 31;
const DSL_FORUM_PEN: [u8; 4] = 3561u32.to_be_bytes();

pub const CALLER_SLOT: usize = 0;
pub const CIRCUIT_SLOT: usize = 1;
pub const REMOTE_SLOT: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidHeader,
    InvalidAttribute,
    InvalidDslForumAttribute,
    Rewrite(rewrite::Error),
}

pub struct Packet<'a> {
    bytes: &'a [u8],
}

impl<'a> Packet<'a> {
    pub fn decode(datagram: &'a [u8]) -> Result<Self, Error> {
        let header = datagram.get(..HEADER_BYTES).ok_or(Error::InvalidHeader)?;
        let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
        if !(HEADER_BYTES..=MAX_PACKET_BYTES).contains(&length) {
            return Err(Error::InvalidHeader);
        }
        let bytes = datagram.get(..length).ok_or(Error::InvalidHeader)?;
        Ok(Self { bytes })
    }

    pub fn wire_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn code(&self) -> u8 {
        self.bytes[0]
    }

    pub fn identifier(&self) -> u8 {
        self.bytes[1]
    }

    pub fn option82_slots(&self) -> Result<Received<'a, 3>, Error> {
        let mut slots = Received::new();
        walk_tlvs(
            &self.bytes[HEADER_BYTES..],
            Error::InvalidAttribute,
            |code, value| match code {
                CALLING_STATION_ID => slots.record(CALLER_SLOT, value).map_err(Error::Rewrite),
                VENDOR_SPECIFIC if value.len() < 5 => Err(Error::InvalidAttribute),
                VENDOR_SPECIFIC if value.starts_with(&DSL_FORUM_PEN) => walk_tlvs(
                    &value[4..],
                    Error::InvalidDslForumAttribute,
                    |id, data| match id {
                        1 => slots.record(CIRCUIT_SLOT, data).map_err(Error::Rewrite),
                        2 => slots.record(REMOTE_SLOT, data).map_err(Error::Rewrite),
                        _ => Ok(()),
                    },
                ),
                _ => Ok(()),
            },
        )?;
        Ok(slots)
    }
}

fn walk_tlvs<'a>(
    mut bytes: &'a [u8],
    invalid: Error,
    mut handle: impl FnMut(u8, &'a [u8]) -> Result<(), Error>,
) -> Result<(), Error> {
    while !bytes.is_empty() {
        let header = bytes.get(..2).ok_or(invalid)?;
        let end = usize::from(header[1]);
        if end < 2 {
            return Err(invalid);
        }
        let value = bytes.get(2..end).ok_or(invalid)?;
        handle(header[0], value)?;
        bytes = &bytes[end..];
    }
    Ok(())
}
