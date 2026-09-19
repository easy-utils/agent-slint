# Easy Agent — Slint (Rust) client. Slint renders via X11/GL/EGL, so a bare
# build host needs the X dev libs (the Makefile is a no-op where they exist).
.PHONY: build run test clean

build:
	cargo build --release

run: build
	./target/release/agent-slint

# `cargo test` builds the whole crate; there are no unit tests yet.
test:
	cargo build --release

clean:
	cargo clean
