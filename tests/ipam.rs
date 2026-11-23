use radiuscore::ipam::{first_free, ipv4_link_subnet, reply_attributes, Error, Prefix, Purpose};

fn prefix(input: &str) -> Prefix {
    input.parse().unwrap()
}

#[test]
fn skips_reserved_and_occupied_ranges_in_both_families() {
    let pool = prefix("192.0.2.0/29");
    let occupied = [
        prefix("192.0.2.7/32"),
        prefix("192.0.2.0/31"),
        prefix("192.0.2.2/32"),
    ];
    assert_eq!(
        first_free(pool, 32, &occupied).unwrap(),
        Some(prefix("192.0.2.3/32"))
    );
    assert_eq!(first_free(pool, 29, &occupied).unwrap(), None);
    assert_eq!(first_free(pool, 28, &occupied), Err(Error::InvalidPrefix));

    let pool = prefix("2001:db8::/32");
    let occupied = [prefix("2001:db8::/48"), prefix("192.0.2.0/24")];
    assert_eq!(
        first_free(pool, 56, &occupied).unwrap(),
        Some(prefix("2001:db8:1::/56"))
    );
    assert_eq!(
        first_free(prefix("::/0"), 0, &[]).unwrap(),
        Some(prefix("::/0"))
    );
    assert_eq!(
        first_free(prefix("::/0"), 64, &[prefix("::/0")]).unwrap(),
        None
    );
}

#[test]
fn distinguishes_ipv4_hosts_ipv6_delegation_and_routed_prefixes() {
    let ip = reply_attributes(prefix("192.0.2.7/32"), Purpose::Host, None).unwrap();
    assert_eq!(
        (ip[0].code, ip[0].value.as_slice()),
        (8, &[192, 0, 2, 7][..])
    );
    let delegated =
        reply_attributes(prefix("2001:db8:100::/56"), Purpose::DelegatedIpv6, None).unwrap();
    assert_eq!(delegated[0].code, 123);
    assert_eq!(delegated[0].value, [0, 56, 0x20, 1, 0x0d, 0xb8, 1, 0, 0]);
    let route = reply_attributes(prefix("198.51.100.32/29"), Purpose::Routed, None).unwrap();
    assert_eq!(route[0].code, 22);
    assert_eq!(route[0].value, b"198.51.100.32/29 0.0.0.0 1");
    let route = reply_attributes(
        prefix("2001:db8:200::/48"),
        Purpose::Routed,
        Some("2001:db8::2".parse().unwrap()),
    )
    .unwrap();
    assert_eq!(route[0].code, 99);
    assert_eq!(route[0].value, b"2001:db8:200::/48 2001:db8::2 1");
    assert_eq!("192.0.2.2/29".parse::<Prefix>(), Err(Error::HostBitsSet));
    assert_eq!(
        reply_attributes(prefix("2001:db8::/48"), Purpose::Routed, None),
        Err(Error::InvalidNextHop)
    );
    let link = ipv4_link_subnet(prefix("192.0.2.0/29"), "192.0.2.3".parse().unwrap()).unwrap();
    assert_eq!(link[1].value, [255, 255, 255, 248]);
    assert!(ipv4_link_subnet(prefix("192.0.2.0/29"), "192.0.2.7".parse().unwrap()).is_err());
}
