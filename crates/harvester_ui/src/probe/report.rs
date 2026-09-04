use std::fs;
use std::path::Path;

use engine_logging::engine_info;
use harvester_ui_bridge::probe::ProbeReport;

pub fn write(root: &Path, report: &ProbeReport) -> std::io::Result<()> {
    let directory = root.join(".local").join("probe");
    fs::create_dir_all(&directory)?;
    let path = directory.join("ipc-report.json");
    let json = serde_json::to_string_pretty(report).expect("probe report is serializable");
    fs::write(&path, json)?;
    engine_info!("[ui-probe] wrote report path={}", path.display());
    Ok(())
}
