# Runtime image for hub. The binary is a static musl build made on the dev machine
# (deploy.ps1: cargo build --release -p hub --target x86_64-unknown-linux-musl), so
# the weak host never compiles Rust. busybox provides wget for the healthcheck.
FROM caddy:2.11.7-alpine AS trust-roots

FROM busybox:1.37.0-musl
# Only public CA roots are copied; the runtime remains the static busybox image.
COPY --from=trust-roots /etc/ssl/certs/ca-certificates.crt /etc/ssl/cert.pem
ENV SSL_CERT_FILE=/etc/ssl/cert.pem
COPY hub /usr/local/bin/hub
RUN mkdir -p /data/chat && chown 65534:65534 /data/chat
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/hub"]
