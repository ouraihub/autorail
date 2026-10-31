//! Pure provider settings helper checks. UI integration remains outside this module.

use super::draft::parse_url;

#[test]
fn accepts_http_urls_and_rejects_credentials() {
    assert_eq!(parse_url(" https://relay.example/v1 ").unwrap(), "https://relay.example/v1");
    assert!(parse_url("ftp://relay.example").is_err());
    assert!(parse_url("https://user:secret@relay.example").is_err());
}
