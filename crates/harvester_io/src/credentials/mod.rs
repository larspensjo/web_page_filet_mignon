//! API keys from Windows Credential Manager (DecisionLog 2026-11-27; Plan.MorningBatchRun K1-K9).
//!
//! A key is read only through a [`StoredKeyAccess`] proof, which only the hosts' `main` can mint
//! from the hidden `--use-stored-keys` launch flag. Without it no process reads a credential.
//! Key values are held in [`ApiKey`], whose `Debug` is redacted and which implements neither
//! `Display` nor `Serialize`. Every failure other than "not found" collapses to one fixed text.

use std::fmt;

use engine_logging::engine_info;

/// Largest credential blob accepted, in bytes (Windows' own limit is 2,560 bytes).
pub const MAX_BLOB_BYTES: usize = 2_560;

/// The generic credentials the Harvester reads. The user name stored with them is ignored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialEntry {
    OpenAi,
    BraveSearch,
}

impl CredentialEntry {
    pub const fn target_name(self) -> &'static str {
        match self {
            Self::OpenAi => "Harvester/OpenAI",
            Self::BraveSearch => "Harvester/BraveSearch",
        }
    }
}

/// How a stored blob was decoded (reported in the key-status line, never the value).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEncoding {
    Utf16Le,
    Utf8,
}

impl KeyEncoding {
    const fn label(self) -> &'static str {
        match self {
            Self::Utf16Le => "utf-16le",
            Self::Utf8 => "utf-8",
        }
    }
}

/// A key value. `Debug` prints only its length; there is no `Display` or `Serialize`.
pub struct ApiKey(String);

impl ApiKey {
    pub(crate) fn from_trusted(value: String) -> Self {
        Self(value)
    }

    /// The raw value, for the provider constructor and the request header only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted {} bytes>", self.0.len())
    }
}

/// The result of reading one entry.
#[derive(Debug)]
pub enum KeyReadOutcome {
    Found {
        key: ApiKey,
        chars: usize,
        encoding: KeyEncoding,
    },
    NotFound,
    Empty,
    /// Any OS failure or undecodable blob. Carries no OS error text (K8).
    Unreadable {
        entry: CredentialEntry,
    },
}

impl KeyReadOutcome {
    /// The fixed failure text for an unreadable entry; `None` for every other outcome.
    pub fn failure_text(&self) -> Option<String> {
        match self {
            Self::Unreadable { entry } => Some(format!(
                "Credential Manager entry {} could not be read",
                entry.target_name()
            )),
            _ => None,
        }
    }

    /// One key-status line for the batch's startup output (K9). Never contains the value.
    pub fn status_line(&self, entry: CredentialEntry) -> String {
        let state = match self {
            Self::Found {
                chars, encoding, ..
            } => format!("found ({chars} chars, {})", encoding.label()),
            Self::NotFound => "not found".to_owned(),
            Self::Empty => "empty".to_owned(),
            Self::Unreadable { .. } => "unreadable".to_owned(),
        };
        format!("[credentials] {}: {state}", entry.target_name())
    }

    fn from_blob(entry: CredentialEntry, blob: Option<Vec<u8>>) -> Self {
        let Some(blob) = blob else {
            return Self::NotFound;
        };
        match decode_blob(&blob) {
            DecodedBlob::Key {
                key,
                chars,
                encoding,
            } => Self::Found {
                key,
                chars,
                encoding,
            },
            DecodedBlob::Empty => Self::Empty,
            DecodedBlob::Unreadable => Self::Unreadable { entry },
        }
    }
}

/// A decoded credential blob.
#[derive(Debug)]
pub enum DecodedBlob {
    Key {
        key: ApiKey,
        chars: usize,
        encoding: KeyEncoding,
    },
    Empty,
    Unreadable,
}

/// Decodes a blob (K6): UTF-16LE when the length is even and every odd byte is zero, otherwise
/// UTF-8; trailing NULs and surrounding ASCII whitespace are trimmed; an empty result is
/// `Empty`; anything that is not printable ASCII, or a blob over [`MAX_BLOB_BYTES`], is
/// `Unreadable`.
pub fn decode_blob(blob: &[u8]) -> DecodedBlob {
    if blob.len() > MAX_BLOB_BYTES {
        return DecodedBlob::Unreadable;
    }
    let utf16 =
        blob.len().is_multiple_of(2) && blob.iter().skip(1).step_by(2).all(|byte| *byte == 0);
    let (text, encoding) = if utf16 {
        let (pairs, _) = blob.as_chunks::<2>();
        let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        match String::from_utf16(&units) {
            Ok(text) => (text, KeyEncoding::Utf16Le),
            Err(_) => return DecodedBlob::Unreadable,
        }
    } else {
        match String::from_utf8(blob.to_vec()) {
            Ok(text) => (text, KeyEncoding::Utf8),
            Err(_) => return DecodedBlob::Unreadable,
        }
    };
    let trimmed = text
        .trim_end_matches('\0')
        .trim_matches(|c: char| c.is_ascii_whitespace());
    if trimmed.is_empty() {
        return DecodedBlob::Empty;
    }
    if !trimmed.chars().all(|c| c.is_ascii_graphic()) {
        return DecodedBlob::Unreadable;
    }
    DecodedBlob::Key {
        chars: trimmed.chars().count(),
        key: ApiKey::from_trusted(trimmed.to_owned()),
        encoding,
    }
}

/// Proof that this process was launched with `--use-stored-keys` (K3). Only the hosts' `main`
/// mints it, from the parsed flag; every credential read requires it.
pub struct StoredKeyAccess(());

impl StoredKeyAccess {
    pub fn from_launch_flag(present: bool) -> Option<Self> {
        present.then_some(Self(()))
    }
}

/// Both entries, read once at host startup.
#[derive(Debug)]
pub struct StoredKeyReads {
    pub openai: KeyReadOutcome,
    pub brave: KeyReadOutcome,
}

/// Reads each entry once. Requires the launch proof.
pub fn read_host_keys(access: &StoredKeyAccess) -> StoredKeyReads {
    StoredKeyReads {
        openai: read_key(access, CredentialEntry::OpenAi),
        brave: read_key(access, CredentialEntry::BraveSearch),
    }
}

fn read_key(_access: &StoredKeyAccess, entry: CredentialEntry) -> KeyReadOutcome {
    let name = entry.target_name();
    // The observable boundary for "no read without the flag" (K4): logged before any OS call.
    engine_info!("[credentials] read requested entry={name}");
    match platform::read(name) {
        Ok(blob) => KeyReadOutcome::from_blob(entry, blob),
        Err(platform::CredentialFailure) => KeyReadOutcome::Unreadable { entry },
    }
}

/// Replaces every occurrence of `key` in `text` with `<redacted>`, including overlapping ones,
/// so an HTTP error body or message that echoes the key cannot leak it (K7).
pub fn scrub(text: &str, key: &ApiKey) -> String {
    let needle = key.expose().as_bytes();
    let haystack = text.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return text.to_owned();
    }
    let mut covered = vec![false; haystack.len()];
    let mut any = false;
    for start in 0..=haystack.len() - needle.len() {
        if &haystack[start..start + needle.len()] == needle {
            covered[start..start + needle.len()].fill(true);
            any = true;
        }
    }
    if !any {
        return text.to_owned();
    }
    // The key is printable ASCII, so covered runs never split a multi-byte character.
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < haystack.len() {
        if covered[index] {
            out.push_str("<redacted>");
            while index < haystack.len() && covered[index] {
                index += 1;
            }
        } else {
            let run_end = covered[index..]
                .iter()
                .position(|c| *c)
                .map_or(haystack.len(), |offset| index + offset);
            out.push_str(&text[index..run_end]);
            index = run_end;
        }
    }
    out
}

#[cfg(all(windows, not(test)))]
mod platform {
    //! The only place that calls the OS credential API.
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_NOT_FOUND},
        Security::Credentials::{CredFree, CredReadW, CRED_TYPE_GENERIC},
    };

    pub(super) struct CredentialFailure;

    pub(super) fn read(name: &str) -> Result<Option<Vec<u8>>, CredentialFailure> {
        let target: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut credential = std::ptr::null_mut();
        // SAFETY: `target` is NUL-terminated and outlives the call; CredReadW allocates the
        // record, which is released with CredFree below.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
            // SAFETY: reads the calling thread's last-error value set by CredReadW.
            return if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                Ok(None)
            } else {
                Err(CredentialFailure)
            };
        }
        // SAFETY: a successful CredReadW returns a valid record whose blob pointer and size are
        // valid until CredFree.
        let blob = unsafe {
            let record = &*credential;
            if record.CredentialBlobSize == 0 || record.CredentialBlob.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(
                    record.CredentialBlob,
                    record.CredentialBlobSize as usize,
                )
                .to_vec()
            }
        };
        // SAFETY: `credential` came from a successful CredReadW and is freed exactly once.
        unsafe { CredFree(credential.cast()) };
        Ok(Some(blob))
    }
}

#[cfg(all(not(windows), not(test)))]
mod platform {
    pub(super) struct CredentialFailure;

    pub(super) fn read(_name: &str) -> Result<Option<Vec<u8>>, CredentialFailure> {
        Ok(None)
    }
}

#[cfg(test)]
mod platform {
    //! Counting stub: unit tests never reach the OS credential store (K5).
    pub(super) struct CredentialFailure;

    pub(super) fn read(name: &str) -> Result<Option<Vec<u8>>, CredentialFailure> {
        super::test_support::record_read(name)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// What the stub returns for one entry.
    #[derive(Clone)]
    pub(crate) enum StubBlob {
        Present(Vec<u8>),
        Missing,
        Failure,
    }

    thread_local! {
        static STUBS: RefCell<HashMap<String, StubBlob>> = RefCell::new(HashMap::new());
        static READS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    }

    pub(crate) fn set_stub(name: &'static str, blob: StubBlob) {
        STUBS.with(|stubs| stubs.borrow_mut().insert(name.to_owned(), blob));
    }

    /// Every entry name read on this thread, in order.
    pub(crate) fn reads() -> Vec<&'static str> {
        READS.with(|reads| reads.borrow().clone())
    }

    pub(super) fn record_read(
        name: &str,
    ) -> Result<Option<Vec<u8>>, super::platform::CredentialFailure> {
        let interned = match name {
            "Harvester/OpenAI" => "Harvester/OpenAI",
            "Harvester/BraveSearch" => "Harvester/BraveSearch",
            other => panic!("unexpected credential entry {other}"),
        };
        READS.with(|reads| reads.borrow_mut().push(interned));
        match STUBS.with(|stubs| stubs.borrow().get(name).cloned()) {
            Some(StubBlob::Present(blob)) => Ok(Some(blob)),
            Some(StubBlob::Failure) => Err(super::platform::CredentialFailure),
            Some(StubBlob::Missing) | None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests;
