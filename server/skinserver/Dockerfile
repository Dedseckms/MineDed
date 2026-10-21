# Drasl image is Nix-built and has no shell or HTTP client, so the healthcheck
# cannot run inside it. Add a static busybox binary used only by the healthcheck
# and by backup.sh (tar). Both base tags are pinned.
FROM busybox:1.37.0-musl AS bb

FROM unmojang/drasl:4.0.1
COPY --from=bb /bin/busybox /busybox
