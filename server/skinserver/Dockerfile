# Drasl image is Nix-built and has no shell or HTTP client, so the healthcheck
# cannot run inside it. Add a static busybox binary used only by the healthcheck
# and by backup.sh (tar). Both base tags are pinned.
FROM busybox:1.37.0-musl AS bb

FROM unmojang/drasl:4.0.1 AS upstream

# MineDed web theme (see theme/README.md): copy Drasl's data dir out of the Nix
# store, verify the overridden upstream files are unchanged, then overlay ours.
FROM busybox:1.37.0-musl AS theme
COPY --from=upstream /nix/store /nix/store
COPY theme/ /theme/
RUN set -eu; \
    src=$(ls -d /nix/store/*/share/drasl | head -n 1); \
    test -f "$src/view/layout.tmpl"; \
    (cd "$src" && sha256sum -c /theme/upstream.sha256); \
    mkdir -p /out; cp -R "$src"/. /out/; chmod -R u+w /out; \
    cp /theme/view/*.tmpl /out/view/; \
    cp /theme/public/* /out/public/

FROM unmojang/drasl:4.0.1
COPY --from=bb /bin/busybox /busybox
COPY --from=theme /out /usr/share/mineded-drasl
# Drasl reads templates, public assets and locales from its DataDirectory.
ENV DRASL_DEFAULT_DATA_DIRECTORY=/usr/share/mineded-drasl
