//! Plan.MorningBatchRun Phase 1 (K3, K5-K8): decode, redaction, proof token, counting stub, scrub.

use super::test_support::{reads, set_stub, StubBlob};
use super::*;

const SENTINEL: &str = "sk-SENTINEL-0123456789abcdef";

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn found_text(outcome: &DecodedBlob) -> (&str, usize, KeyEncoding) {
    match outcome {
        DecodedBlob::Key {
            key,
            chars,
            encoding,
        } => (key.expose(), *chars, *encoding),
        other => panic!("expected a key, got {other:?}"),
    }
}

#[test]
fn decodes_utf16le_ascii_blob() {
    let decoded = decode_blob(&utf16("sk-abc123"));
    assert_eq!(found_text(&decoded), ("sk-abc123", 9, KeyEncoding::Utf16Le));
}

#[test]
fn decodes_utf8_ascii_blob() {
    let decoded = decode_blob(b"sk-abc123");
    assert_eq!(found_text(&decoded), ("sk-abc123", 9, KeyEncoding::Utf8));
}

#[test]
fn even_length_utf8_blob_is_not_mistaken_for_utf16() {
    let decoded = decode_blob(b"sk-abcd1");
    assert_eq!(found_text(&decoded), ("sk-abcd1", 8, KeyEncoding::Utf8));
}

#[test]
fn trailing_nuls_are_trimmed_in_both_encodings() {
    let mut wide = utf16("sk-wide");
    wide.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(found_text(&decode_blob(&wide)).0, "sk-wide");
    assert_eq!(found_text(&decode_blob(b"sk-narrow\0\0")).0, "sk-narrow");
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    assert_eq!(
        found_text(&decode_blob(&utf16(" \tsk-pad\r\n"))),
        ("sk-pad", 6, KeyEncoding::Utf16Le)
    );
    assert_eq!(found_text(&decode_blob(b"  sk-pad \n")).0, "sk-pad");
}

#[test]
fn empty_and_whitespace_only_blobs_are_empty() {
    assert!(matches!(decode_blob(b""), DecodedBlob::Empty));
    assert!(matches!(decode_blob(b" \t\r\n"), DecodedBlob::Empty));
    assert!(matches!(decode_blob(&utf16("   ")), DecodedBlob::Empty));
    assert!(matches!(decode_blob(&[0, 0]), DecodedBlob::Empty));
}

#[test]
fn non_graphic_surrogate_and_oversized_blobs_are_unreadable() {
    assert!(matches!(
        decode_blob(b"sk-has space"),
        DecodedBlob::Unreadable
    ));
    assert!(matches!(
        decode_blob("sk-é".as_bytes()),
        DecodedBlob::Unreadable
    ));
    assert!(matches!(
        decode_blob(&[0xff, 0xfe, 0x41]),
        DecodedBlob::Unreadable
    ));
    // An unpaired high surrogate (0xD800) in UTF-16LE.
    let mut surrogate = utf16("sk-");
    surrogate.extend_from_slice(&[0x00, 0xd8]);
    assert!(matches!(decode_blob(&surrogate), DecodedBlob::Unreadable));
    let oversized = vec![b'a'; MAX_BLOB_BYTES + 1];
    assert!(matches!(decode_blob(&oversized), DecodedBlob::Unreadable));
    let at_limit = vec![b'a'; MAX_BLOB_BYTES];
    assert!(matches!(decode_blob(&at_limit), DecodedBlob::Key { .. }));
}

#[test]
fn api_key_debug_is_redacted_with_byte_count() {
    let key = ApiKey::from_trusted(SENTINEL.to_owned());
    let debug = format!("{key:?}");
    assert_eq!(debug, format!("<redacted {} bytes>", SENTINEL.len()));
    assert!(!debug.contains(SENTINEL));
    let outcome = KeyReadOutcome::Found {
        key,
        chars: SENTINEL.len(),
        encoding: KeyEncoding::Utf8,
    };
    assert!(!format!("{outcome:?}").contains(SENTINEL));
}

#[test]
fn unreadable_outcome_carries_only_the_fixed_text() {
    let outcome = KeyReadOutcome::Unreadable {
        entry: CredentialEntry::OpenAi,
    };
    assert_eq!(
        outcome.failure_text().as_deref(),
        Some("Credential Manager entry Harvester/OpenAI could not be read")
    );
    assert!(!format!("{outcome:?}").contains(SENTINEL));
    assert_eq!(KeyReadOutcome::NotFound.failure_text(), None);
}

// Compile-time proof that `ApiKey` implements neither `Display` nor `Serialize`: if it did, the
// call below would be ambiguous between the two blanket impls and fail to compile.
trait AmbiguousIfDisplay<A> {
    fn check() {}
}
impl<T: ?Sized> AmbiguousIfDisplay<()> for T {}
#[allow(dead_code)]
struct ImplsDisplay;
impl<T: ?Sized + std::fmt::Display> AmbiguousIfDisplay<ImplsDisplay> for T {}

trait AmbiguousIfSerialize<A> {
    fn check() {}
}
impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
#[allow(dead_code)]
struct ImplsSerialize;
impl<T: ?Sized + serde::Serialize> AmbiguousIfSerialize<ImplsSerialize> for T {}

#[test]
fn api_key_implements_neither_display_nor_serialize() {
    <ApiKey as AmbiguousIfDisplay<_>>::check();
    <ApiKey as AmbiguousIfSerialize<_>>::check();
}

#[test]
fn launch_flag_absent_mints_no_proof() {
    assert!(StoredKeyAccess::from_launch_flag(false).is_none());
    assert!(StoredKeyAccess::from_launch_flag(true).is_some());
}

#[test]
fn entries_use_the_agreed_target_names() {
    assert_eq!(CredentialEntry::OpenAi.target_name(), "Harvester/OpenAI");
    assert_eq!(
        CredentialEntry::BraveSearch.target_name(),
        "Harvester/BraveSearch"
    );
}

#[test]
fn read_host_keys_reads_each_entry_once_through_the_proof() {
    set_stub("Harvester/OpenAI", StubBlob::Present(utf16(SENTINEL)));
    set_stub("Harvester/BraveSearch", StubBlob::Missing);
    let access = StoredKeyAccess::from_launch_flag(true).expect("flag present");
    let before = reads().len();

    let keys = read_host_keys(&access);

    assert_eq!(
        &reads()[before..],
        &["Harvester/OpenAI", "Harvester/BraveSearch"]
    );
    match &keys.openai {
        KeyReadOutcome::Found {
            key,
            chars,
            encoding,
        } => {
            assert_eq!(key.expose(), SENTINEL);
            assert_eq!(*chars, SENTINEL.len());
            assert_eq!(*encoding, KeyEncoding::Utf16Le);
        }
        other => panic!("expected the OpenAI key, got {other:?}"),
    }
    assert!(matches!(keys.brave, KeyReadOutcome::NotFound));
}

#[test]
fn os_failure_and_empty_blob_map_to_fixed_outcomes() {
    set_stub("Harvester/OpenAI", StubBlob::Failure);
    set_stub("Harvester/BraveSearch", StubBlob::Present(Vec::new()));
    let access = StoredKeyAccess::from_launch_flag(true).expect("flag present");

    let keys = read_host_keys(&access);

    assert!(matches!(
        keys.openai,
        KeyReadOutcome::Unreadable {
            entry: CredentialEntry::OpenAi
        }
    ));
    assert!(matches!(keys.brave, KeyReadOutcome::Empty));
}

#[test]
fn status_line_names_entry_and_never_the_value() {
    let found = KeyReadOutcome::Found {
        key: ApiKey::from_trusted(SENTINEL.to_owned()),
        chars: SENTINEL.len(),
        encoding: KeyEncoding::Utf16Le,
    };
    let line = found.status_line(CredentialEntry::OpenAi);
    assert_eq!(
        line,
        format!(
            "[credentials] Harvester/OpenAI: found ({} chars, utf-16le)",
            SENTINEL.len()
        )
    );
    assert!(!line.contains(SENTINEL));
    assert_eq!(
        KeyReadOutcome::NotFound.status_line(CredentialEntry::BraveSearch),
        "[credentials] Harvester/BraveSearch: not found"
    );
    assert_eq!(
        KeyReadOutcome::Empty.status_line(CredentialEntry::BraveSearch),
        "[credentials] Harvester/BraveSearch: empty"
    );
    assert_eq!(
        KeyReadOutcome::Unreadable {
            entry: CredentialEntry::OpenAi
        }
        .status_line(CredentialEntry::OpenAi),
        "[credentials] Harvester/OpenAI: unreadable"
    );
}

#[test]
fn scrub_replaces_every_occurrence_and_keeps_other_text() {
    let key = ApiKey::from_trusted(SENTINEL.to_owned());
    let text = format!("error: key {SENTINEL} rejected; echo={SENTINEL}{SENTINEL}.");
    let scrubbed = scrub(&text, &key);
    assert_eq!(scrubbed, "error: key <redacted> rejected; echo=<redacted>.");
    assert!(!scrubbed.contains(SENTINEL));
    assert_eq!(scrub("nothing to see", &key), "nothing to see");
}

#[test]
fn scrub_covers_overlapping_occurrences() {
    let key = ApiKey::from_trusted("abab".to_owned());
    // "ababab" holds overlapping matches at offsets 0 and 2; no residue may remain.
    let scrubbed = scrub("x ababab y", &key);
    assert_eq!(scrubbed, "x <redacted> y");
    assert!(!scrubbed.contains("abab"));
}
