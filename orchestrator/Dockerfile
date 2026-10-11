FROM rust:1-alpine AS builder

WORKDIR /app
RUN apk add --no-cache musl-dev
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release

FROM scratch AS artifact

COPY --from=builder /app/target/release/orchestrator /orchestrator

FROM alpine:3.22

ENV HOME=/home/relayfold

LABEL org.opencontainers.image.source="https://github.com/parsablelabs/relayfold" \
      org.opencontainers.image.description="RelayFold orchestrator" \
      org.opencontainers.image.licenses="MIT"

RUN apk add --no-cache ca-certificates \
    && addgroup -g 10001 relayfold \
    && adduser -D -h /home/relayfold -u 10001 -G relayfold relayfold \
    && mkdir -p /var/lib/relayfold/oauth /workspaces \
    && chown relayfold:relayfold /var/lib/relayfold /var/lib/relayfold/oauth /workspaces
COPY --from=builder /app/target/release/orchestrator /usr/local/bin/orchestrator
USER relayfold

EXPOSE 3000 3001
ENTRYPOINT ["/usr/local/bin/orchestrator"]

CMD ["start"]
