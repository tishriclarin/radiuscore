use radiuscore::option82::{decode_dhcp_option, decode_escaped_log, decode_hex, hex, Error};

#[test]
fn public_debug_values_round_trip_without_assuming_a_vendor_layout() {
    for row in include_str!("fixtures/option82-public.tsv").lines() {
        if row.starts_with('#') {
            continue;
        }
        let columns: Vec<_> = row.split('\t').collect();
        let [name, encoding, input, expected] = columns.as_slice() else {
            panic!("invalid fixture row: {row}");
        };
        let decoded = match *encoding {
            "hex" => decode_hex(input),
            "octal" => decode_escaped_log(input),
            _ => panic!("invalid encoding: {encoding}"),
        };
        assert_eq!(&hex(&decoded.unwrap()), expected, "{name}");
    }
}

#[test]
fn dhcp_decoder_preserves_binary_ids_and_rejects_ambiguous_input() {
    let bytes = decode_hex("02074d595f41505f310106500000020005").unwrap();
    let decoded = decode_dhcp_option(&bytes).unwrap();
    assert_eq!(hex(decoded.circuit_id.unwrap()), "500000020005");
    assert_eq!(decoded.remote_id, Some(b"MY_AP_1".as_slice()));
    assert_eq!(decode_dhcp_option(&[1, 2, 3]), Err(Error::Truncated));
    assert_eq!(
        decode_dhcp_option(&[1, 0, 1, 0]),
        Err(Error::DuplicateCircuitId)
    );
    assert_eq!(
        decode_dhcp_option(&[2, 0, 2, 0]),
        Err(Error::DuplicateRemoteId)
    );
    assert_eq!(decode_dhcp_option(&[0; 256]), Err(Error::TooLong));
    assert_eq!(decode_hex("0xabc"), Err(Error::InvalidHex));
    assert_eq!(decode_escaped_log(r"\400"), Err(Error::InvalidEscape));
}
