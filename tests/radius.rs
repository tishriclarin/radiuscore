use radiuscore::{
    option82::decode_hex,
    radius::{Error, Packet, CALLER_SLOT, CIRCUIT_SLOT, REMOTE_SLOT},
    rewrite::RewritePlan,
};

#[test]
fn decodes_option82_vsa_and_late_guard_in_one_pass() {
    // Public diagnostic values wrapped in a synthetic RADIUS packet.
    let raw = decode_hex(concat!(
        "0137003800000000000000000000000000000000",
        "1a1100000de902094d595f41505f310100",
        "1f1330303a31313a32323a61613a62623a6363",
    ))
    .unwrap();
    let malformed = Packet::decode(&raw).unwrap();
    assert!(matches!(
        malformed.option82_slots(),
        Err(Error::InvalidDslForumAttribute)
    ));

    let raw = decode_hex(concat!(
        "0137003e00000000000000000000000000000000",
        "1a1700000de902094d595f41505f310108500000020005",
        "1f1330303a31313a32323a61613a62623a6363",
    ))
    .unwrap();
    let packet = Packet::decode(&raw).unwrap();
    assert_eq!(packet.code(), 1);
    assert_eq!(packet.identifier(), 55);
    let slots = packet.option82_slots().unwrap();
    RewritePlan::<3>::compile(&[])
        .unwrap()
        .worker()
        .then_lookup(&slots, |working| {
            assert_eq!(
                working.attribute(CALLER_SLOT).unwrap(),
                Some(b"00:11:22:aa:bb:cc".as_slice())
            );
            assert_eq!(
                working.attribute(REMOTE_SLOT).unwrap(),
                Some(b"MY_AP_1".as_slice())
            );
            assert_eq!(
                working.attribute(CIRCUIT_SLOT).unwrap(),
                Some(&[0x50, 0, 0, 2, 0, 5][..])
            );
        })
        .unwrap();
}

#[test]
fn rejects_incomplete_outer_attribute_and_ignores_transport_padding() {
    let mut raw = vec![0_u8; 23];
    raw[0] = 1;
    raw[3] = 21;
    raw[20] = 31;
    let packet = Packet::decode(&raw).unwrap();
    assert_eq!(packet.wire_bytes().len(), 21);
    assert!(matches!(
        packet.option82_slots(),
        Err(Error::InvalidAttribute)
    ));
    raw[3] = 24;
    assert!(matches!(Packet::decode(&raw), Err(Error::InvalidHeader)));
}
