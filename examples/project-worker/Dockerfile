ARG RELAYFOLD_WORKER_BASE_IMAGE=relayfold-worker:local
FROM ${RELAYFOLD_WORKER_BASE_IMAGE} AS project-dependencies

USER root
RUN mkdir -p /opt/relayfold-project-deps \
    && chown relayfold:relayfold /opt/relayfold-project-deps
COPY --chown=relayfold:relayfold worker/package.json worker/package-lock.json /opt/relayfold-project-deps/

USER relayfold
RUN npm ci --prefix /opt/relayfold-project-deps --include=dev --cache /tmp/npm-cache

FROM ${RELAYFOLD_WORKER_BASE_IMAGE}

USER root
RUN apk add --no-cache ripgrep
COPY --from=project-dependencies /opt/relayfold-project-deps /opt/relayfold-project-deps

USER relayfold
