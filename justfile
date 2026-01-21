# Tasks for development workflow

default := ["build"]

build:
	cargo build --workspace

test:
	cargo test --workspace

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets
