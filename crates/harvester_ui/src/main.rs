fn main() {
    let probe = std::env::args()
        .skip(1)
        .any(|argument| argument == "--probe-ipc");
    if let Err(error) = harvester_ui::run(probe) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
