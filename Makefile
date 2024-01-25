.PHONY: fabric-rs
ARCH?=x86_64
all: fabric-rs

fabric-rs: ebpf
	(cd fabric-rs; RUSTFLAGS="--cfg s2n_quic_unstable" cargo build --release --target=${ARCH}-unknown-linux-gnu)
ebpf:
	(cargo xtask build-ebpf --release)