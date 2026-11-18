//! The signature of a webhook delivery, against values that another
//! implementation computed.

use unsup_gateway::webhook::sign::{hmac_sha256, sign, verify};

fn hex(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn hmac_sha256_gives_the_values_of_rfc_4231() {
    // Test cases 1, 2, 3, 4, 6 and 7 of RFC 4231. Case 5 cuts the
    // output, which this code never does.
    let long_key = [0xaa_u8; 131];
    let counted_key: Vec<u8> = (1..=25).collect();
    let cases: [(&[u8], &[u8], &str); 6] = [
        (
            &[0x0b; 20],
            b"Hi There",
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        ),
        (
            b"Jefe",
            b"what do ya want for nothing?",
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        ),
        (
            &[0xaa; 20],
            &[0xdd; 50],
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
        ),
        (
            &counted_key,
            &[0xcd; 50],
            "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
        ),
        // A key that is longer than one block of the hash.
        (
            &long_key,
            b"Test Using Larger Than Block-Size Key - Hash Key First",
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
        ),
        (
            &long_key,
            b"This is a test using a larger than block-size key and a larger than block-size \
              data. The key needs to be hashed before being used by the HMAC algorithm.",
            "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
        ),
    ];
    for (number, (key, message, expected)) in cases.into_iter().enumerate() {
        assert_eq!(hex(hmac_sha256(key, message)), expected, "case {number}");
    }
    // A key of exactly one block is used as it is. The value comes from
    // the `hmac` module of Python.
    assert_eq!(
        hex(hmac_sha256(&[b'k'; 64], b"exactly one block of key")),
        "baa93ea4ccc7062ed6870c5e7937dbd35ad7791df4e29df9addeb429ff491acf"
    );
}

#[test]
fn the_signature_header_is_the_hmac_of_version_time_and_body() {
    // The value comes from the `hmac` module of Python, for the key
    // `unsup_whs_test_secret` and the text `v1:1700000000:<body>`.
    let body = br#"{"resident":7,"notices":[],"reports":[],"next_notice":1}"#;
    let expected = "v1=f8726c8463cce3c2975ceb17f6a638d49cd7454502c2733d180b79bd5674650a";
    assert_eq!(sign("unsup_whs_test_secret", 1_700_000_000, body), expected);

    assert!(verify("unsup_whs_test_secret", 1_700_000_000, body, expected));
    // Another secret, another time, another body or another signature
    // does not verify.
    assert!(!verify("unsup_whs_other_secret", 1_700_000_000, body, expected));
    assert!(!verify("unsup_whs_test_secret", 1_700_000_001, body, expected));
    assert!(!verify("unsup_whs_test_secret", 1_700_000_000, &body[1..], expected));
    assert!(!verify("unsup_whs_test_secret", 1_700_000_000, body, &expected[..66]));
    assert!(!verify("unsup_whs_test_secret", 1_700_000_000, body, &expected.replace("v1=", "v2=")));
    assert!(!verify("unsup_whs_test_secret", 1_700_000_000, body, &expected.to_uppercase()));
}
