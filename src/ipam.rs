use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidPrefix,
    HostBitsSet,
    IncompatibleReply,
    InvalidNextHop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefix {
    address: IpAddr,
    length: u8,
}

impl Prefix {
    pub fn new(address: IpAddr, length: u8) -> Result<Self, Error> {
        let prefix = Self { address, length };
        if length > prefix.bits() {
            return Err(Error::InvalidPrefix);
        }
        if prefix.number() & prefix.host_mask() != 0 {
            return Err(Error::HostBitsSet);
        }
        Ok(prefix)
    }

    pub fn address(self) -> IpAddr {
        self.address
    }

    pub fn length(self) -> u8 {
        self.length
    }

    fn bits(self) -> u8 {
        if self.address.is_ipv4() {
            32
        } else {
            128
        }
    }

    fn number(self) -> u128 {
        match self.address {
            IpAddr::V4(address) => u128::from(u32::from(address)),
            IpAddr::V6(address) => u128::from(address),
        }
    }

    fn host_mask(self) -> u128 {
        match self.bits() - self.length {
            128 => u128::MAX,
            count => (1u128 << count) - 1,
        }
    }

    fn last(self) -> u128 {
        self.number() | self.host_mask()
    }

    fn at(self, number: u128) -> IpAddr {
        if self.address.is_ipv4() {
            IpAddr::V4(Ipv4Addr::from(number as u32))
        } else {
            IpAddr::V6(Ipv6Addr::from(number))
        }
    }

    pub fn contains(self, other: Self) -> bool {
        self.bits() == other.bits()
            && self.number() <= other.number()
            && self.last() >= other.last()
    }

    pub fn overlaps(self, other: Self) -> bool {
        self.bits() == other.bits()
            && self.number() <= other.last()
            && other.number() <= self.last()
    }
}

impl fmt::Display for Prefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.address, self.length)
    }
}

impl FromStr for Prefix {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (address, length) = value.split_once('/').ok_or(Error::InvalidPrefix)?;
        Self::new(
            address.parse().map_err(|_| Error::InvalidPrefix)?,
            length.parse().map_err(|_| Error::InvalidPrefix)?,
        )
    }
}

/// Pick a free, aligned prefix by skipping occupied ranges in ascending order.
/// In a live allocator, hold the pool row lock until the chosen assignment is inserted.
pub fn first_free(pool: Prefix, length: u8, occupied: &[Prefix]) -> Result<Option<Prefix>, Error> {
    if length < pool.length || length > pool.bits() {
        return Err(Error::InvalidPrefix);
    }
    let mut candidate = Prefix::new(pool.address, length)?;
    let mut occupied: Vec<_> = occupied
        .iter()
        .copied()
        .filter(|item| pool.overlaps(*item))
        .collect();
    occupied.sort_unstable_by_key(|prefix| prefix.number());
    for item in occupied {
        if candidate.last() < item.number() {
            break;
        }
        if item.last() >= candidate.number() {
            let Some(next) = (item.last() | candidate.host_mask()).checked_add(1) else {
                return Ok(None);
            };
            if next > pool.last() {
                return Ok(None);
            }
            candidate = Prefix::new(pool.at(next), length)?;
        }
    }
    Ok(Some(candidate))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Host,
    Link,
    DelegatedIpv6,
    Routed,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ReplyAttribute {
    pub code: u8,
    pub value: Vec<u8>,
}

fn ipv6_prefix_attribute(code: u8, address: Ipv6Addr, length: u8) -> ReplyAttribute {
    let bytes = address.octets();
    let octets = usize::from(length).div_ceil(8);
    let mut value = Vec::with_capacity(octets + 2);
    value.extend([0, length]);
    value.extend_from_slice(&bytes[..octets]);
    ReplyAttribute { code, value }
}

/// Return standard RADIUS reply values. The caller gates purposes by NAS capabilities.
pub fn reply_attributes(
    prefix: Prefix,
    purpose: Purpose,
    next_hop: Option<IpAddr>,
) -> Result<Vec<ReplyAttribute>, Error> {
    if purpose != Purpose::Routed && next_hop.is_some() {
        return Err(Error::InvalidNextHop);
    }
    let attributes = match (prefix.address, purpose) {
        (IpAddr::V4(address), Purpose::Host) if prefix.length == 32 => {
            vec![ReplyAttribute {
                code: 8,
                value: address.octets().to_vec(),
            }]
        }
        (IpAddr::V6(address), Purpose::Host) if prefix.length == 128 => {
            vec![ReplyAttribute {
                code: 168,
                value: address.octets().to_vec(),
            }]
        }
        (IpAddr::V6(address), Purpose::Link) if prefix.length < 128 => {
            vec![ipv6_prefix_attribute(97, address, prefix.length)]
        }
        (IpAddr::V6(address), Purpose::DelegatedIpv6) if prefix.length < 128 => {
            vec![ipv6_prefix_attribute(123, address, prefix.length)]
        }
        (IpAddr::V4(_), Purpose::Routed) => {
            let gateway = match next_hop {
                Some(IpAddr::V4(gateway)) => gateway,
                None => Ipv4Addr::UNSPECIFIED,
                _ => return Err(Error::InvalidNextHop),
            };
            vec![ReplyAttribute {
                code: 22,
                value: format!("{prefix} {gateway} 1").into_bytes(),
            }]
        }
        (IpAddr::V6(_), Purpose::Routed) => {
            let Some(IpAddr::V6(gateway)) = next_hop else {
                return Err(Error::InvalidNextHop);
            };
            if gateway.is_unspecified() {
                return Err(Error::InvalidNextHop);
            }
            vec![ReplyAttribute {
                code: 99,
                value: format!("{prefix} {gateway} 1").into_bytes(),
            }]
        }
        _ => return Err(Error::IncompatibleReply),
    };
    Ok(attributes)
}

/// Render a NAS-managed IPv4 on-link subnet for a specific host in that subnet.
pub fn ipv4_link_subnet(prefix: Prefix, host: Ipv4Addr) -> Result<Vec<ReplyAttribute>, Error> {
    let IpAddr::V4(network) = prefix.address else {
        return Err(Error::IncompatibleReply);
    };
    if prefix.length == 32
        || !prefix.contains(Prefix::new(IpAddr::V4(host), 32)?)
        || (prefix.length < 31 && (host == network || u128::from(u32::from(host)) == prefix.last()))
    {
        return Err(Error::IncompatibleReply);
    }
    let netmask = !((prefix.host_mask()) as u32);
    Ok(vec![
        ReplyAttribute {
            code: 8,
            value: host.octets().to_vec(),
        },
        ReplyAttribute {
            code: 9,
            value: netmask.to_be_bytes().to_vec(),
        },
    ])
}
