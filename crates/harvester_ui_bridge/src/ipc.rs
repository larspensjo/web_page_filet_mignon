use engine_logging::engine_warn;
use harvester_core::UiIntent;

pub const IPC_SCHEMA_VERSION: u32 = 7;

pub fn decode_intent(command_name: &str, payload: &[u8]) -> Result<UiIntent, serde_json::Error> {
    serde_json::from_slice(payload).map_err(|error| {
        engine_warn!(
            "[ui-ipc] rejected command={command_name} payload_len={} error={error}",
            payload.len()
        );
        error
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvester_core::UiIntent;

    #[test]
    fn rejects_bad_ipc_without_panicking() {
        for payload in [
            b"{".as_slice(),
            br#"{"type":"Unknown"}"#,
            br#"{"type":"PollSources","extra":1}"#,
            br#"{"type":"SelectJob","payload":{"job_id":1,"extra":1}}"#,
            br#"{"type":"SelectJob","payload":{"job_id":1},"extra":1}"#,
            br#"[]"#,
        ] {
            assert!(decode_intent("dispatch_intent", payload).is_err());
        }
    }

    #[test]
    fn round_trips_intent_json() {
        let intent = UiIntent::OpenExtractedLink {
            job_id: 4,
            link_index: 2,
        };
        let encoded = serde_json::to_vec(&intent).unwrap();
        assert_eq!(decode_intent("dispatch_intent", &encoded).unwrap(), intent);
    }

    #[test]
    fn decodes_last_24_hours_job_list_mode() {
        let payload = br#"{"type":"SetJobListMode","payload":{"mode":"Last24Hours"}}"#;
        assert_eq!(
            decode_intent("dispatch_intent", payload).unwrap(),
            UiIntent::SetJobListMode {
                mode: harvester_core::JobListMode::Last24Hours,
            }
        );
    }

    #[test]
    fn schema_version_matches_frontend_constant() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../frontend/src/ipc/schemaVersion.ts");
        let source = std::fs::read_to_string(path).expect("schemaVersion.ts must exist");
        let parsed = source
            .trim()
            .strip_prefix("export const IPC_SCHEMA_VERSION = ")
            .and_then(|line| line.strip_suffix(';'))
            .and_then(|number| number.parse::<u32>().ok());
        assert_eq!(parsed, Some(IPC_SCHEMA_VERSION));
    }
}
