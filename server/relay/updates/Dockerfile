# Update mirror worker: alpine + curl + jq running mirror.sh as nobody.
# /data is pre-created with nobody ownership so a fresh named volume inherits it.
FROM alpine:3.20.3
RUN apk add --no-cache curl jq \
 && mkdir -p /data/www /data/tmp \
 && chown -R 65534:65534 /data
COPY mirror.sh /usr/local/bin/mirror.sh
USER 65534:65534
ENTRYPOINT ["/bin/sh", "/usr/local/bin/mirror.sh"]
