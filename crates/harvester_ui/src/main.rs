use harvester_io::credentials::StoredKeyAccess;

fn main() {
    let mut probe = false;
    let mut use_stored_keys = false;
    for argument in std::env::args_os().skip(1) {
        probe |= argument == "--probe-ipc";
        use_stored_keys |= argument == "--use-stored-keys";
    }
    let stored_keys = StoredKeyAccess::from_launch_flag(use_stored_keys);
    if let Err(error) = harvester_ui::run(probe, stored_keys) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
