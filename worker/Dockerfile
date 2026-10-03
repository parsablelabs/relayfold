FROM node:24-alpine AS builder

WORKDIR /app

# Copy package.json and tsconfig.json
COPY package*.json ./
COPY tsconfig.json ./

# Install dependencies (ignoring scripts for security if needed, but simple install is fine for skeleton)
RUN npm install

# Copy source code
COPY src ./src

# Build the TypeScript code
RUN npm run build

# Stage 2: Create the production image
FROM node:24-alpine

WORKDIR /app
ARG RELAYFOLD_PI_PACKAGES=""
ENV NODE_ENV=production
ENV HOME=/home/relayfold
ENV XDG_CACHE_HOME=/home/relayfold/.cache
ENV npm_config_cache=/home/relayfold/.cache/npm
ENV COREPACK_HOME=/home/relayfold/.cache/corepack

LABEL org.opencontainers.image.source="https://github.com/parsablelabs/relayfold" \
      org.opencontainers.image.description="RelayFold worker" \
      org.opencontainers.image.licenses="MIT"

RUN apk add --no-cache bash curl git github-cli make g++ python3 py3-pip qemu-system-aarch64 qemu-system-x86_64 qemu-img
RUN addgroup -g 10001 relayfold \
    && adduser -D -h /home/relayfold -u 10001 -G relayfold relayfold \
    && mkdir -p /app /home/relayfold/.relayfold /home/relayfold/.cache /home/relayfold/.pi/agent/skills /workspaces \
    && chown relayfold:relayfold /app /home/relayfold /home/relayfold/.relayfold /home/relayfold/.cache /home/relayfold/.pi /workspaces

# Copy package.json and install production dependencies only
COPY --chown=relayfold:relayfold package*.json ./

# Populate the runtime npm cache with the same owner used during task execution.
USER relayfold
RUN npm install --omit=dev
RUN if [ -n "$RELAYFOLD_PI_PACKAGES" ]; then npm install --omit=dev --no-save $RELAYFOLD_PI_PACKAGES; fi
USER root
RUN corepack enable

# Copy the built output from the builder stage
COPY --from=builder --chown=relayfold:relayfold /app/dist ./dist

# Set the entrypoint to run the worker directly with node
USER relayfold
ENTRYPOINT ["node", "dist/index.js"]
