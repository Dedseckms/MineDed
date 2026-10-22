# Runtime image for hub. The binary is a static musl build made on the dev machine
# (deploy.ps1: cargo build --release -p hub --target x86_64-unknown-linux-musl), so
# the weak host never compiles Rust. busybox provides wget for the healthcheck.
FROM busybox:1.37.0-musl
COPY hub /usr/local/bin/hub
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/hub"]
